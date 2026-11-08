//! The relations of an answer as a graph. Entities, relations and literals are nodes that a person
//! can select, drag and expand, on a canvas that pans and zooms. Every node is a control with a
//! spoken name, so a screen reader reaches the graph as well as the eye does, and the list holds
//! the same relations exactly.
mod layout;
mod model;

use crate::results;
use crate::theme::{self, Palette};
use crate::widgets;
use eframe::egui::{
    self, accesskit, vec2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Galley, Key,
    Pos2, Rect, Scene, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, WidgetInfo,
    WidgetType,
};
use layout::Layout;
use model::{Kind, Model};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

/// What is selected is shown beside the canvas, like an inspector, when the window is this wide;
/// in a narrower one it goes under the canvas, which then leaves it this much room.
const WIDE: f32 = 760.0;
const INSPECTOR: f32 = 300.0;
const DETAIL_HEIGHT: f32 = 190.0;
const MIN_CANVAS: f32 = 260.0;
/// Room around the graph when it is fitted to the canvas.
const MARGIN: f32 = 36.0;
/// The crossbar inside a relation node.
const BAR: f32 = 16.0;
/// A new graph is mostly arranged before it is first drawn, so it does not swirl into place.
const FIRST_BUDGET: Duration = Duration::from_millis(40);
const FRAME_BUDGET: Duration = Duration::from_millis(6);
/// What steps back when something is selected, and how much a stale relation fades.
const DIM: f32 = 0.3;
const STALE: f32 = 0.6;

/// What the person asked for that the graph cannot do alone.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Add the relations of this entity.
    Expand(String),
    /// Show this relation in the list.
    ShowInList(String),
}

pub struct Graph {
    model: Model,
    layout: Layout,
    selected: Option<String>,
    scene: Rect,
    /// Keep the whole graph in view until the person pans or zooms.
    follow: bool,
    /// The model changed since the layout last saw it.
    changed: bool,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            model: Model::default(),
            layout: Layout::default(),
            selected: None,
            // An empty rect makes the scene fit whatever it is first given.
            scene: Rect::ZERO,
            follow: true,
            changed: false,
        }
    }
}

/// A node's text, measured once a frame. Its color is chosen when it is painted.
struct Look {
    main: Arc<Galley>,
    caption: Option<Arc<Galley>>,
    size: Vec2,
}

const PLACEHOLDER: Color32 = Color32::PLACEHOLDER;

impl Look {
    fn measure(ui: &Ui, node: &model::Node) -> Self {
        let painter = ui.painter();
        match node.kind {
            Kind::Entity => {
                let main =
                    painter.layout_no_wrap(node.label.clone(), theme::headline(), PLACEHOLDER);
                let caption = node.caption.as_ref().map(|caption| {
                    painter.layout_no_wrap(caption.clone(), theme::subheadline(), PLACEHOLDER)
                });
                let width = caption
                    .as_ref()
                    .map_or(main.size().x, |c| c.size().x.max(main.size().x));
                let height = main.size().y + caption.as_ref().map_or(0.0, |c| c.size().y);
                Self {
                    main,
                    caption,
                    size: vec2(width + 20.0, height + 12.0),
                }
            }
            Kind::Relation => {
                let font = FontId::new(12.0, theme::medium());
                let main = painter.layout_no_wrap(node.label.clone(), font, PLACEHOLDER);
                Self {
                    size: vec2(
                        BAR + 8.0 + main.size().x + 22.0,
                        main.size().y.max(12.0) + 12.0,
                    ),
                    main,
                    caption: None,
                }
            }
            Kind::Literal => {
                let main =
                    painter.layout_no_wrap(node.label.clone(), theme::subheadline(), PLACEHOLDER);
                Self {
                    size: main.size() + vec2(16.0, 8.0),
                    main,
                    caption: None,
                }
            }
        }
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// Where the line between two boxes leaves the first one.
fn edge(centre: Pos2, half: Vec2, toward: Pos2) -> Pos2 {
    let d = toward - centre;
    if d.x.abs() < f32::EPSILON && d.y.abs() < f32::EPSILON {
        return centre;
    }
    let scale = (half.x / d.x.abs().max(f32::EPSILON)).min(half.y / d.y.abs().max(f32::EPSILON));
    centre + d * scale.min(1.0)
}

impl Graph {
    /// A new answer: a new graph, arranged from nothing.
    pub fn show(&mut self, value: &Value) {
        *self = Self {
            model: Model::of(value).unwrap_or_default(),
            follow: true,
            changed: true,
            ..Self::default()
        };
    }

    /// The same graph grown by an expansion: what was there keeps its place and selection.
    pub fn update(&mut self, value: &Value) {
        self.model = Model::of(value).unwrap_or_default();
        if let Some(key) = &self.selected {
            if self.model.find(key).is_none() {
                self.selected = None;
            }
        }
        self.changed = true;
    }

    /// The relation selected in the graph, so the list can mark it.
    pub fn selected_relation(&self) -> Option<&str> {
        let key = self.selected.as_deref()?;
        let node = &self.model.nodes[self.model.find(key)?];
        (node.kind == Kind::Relation).then_some(key)
    }

    #[cfg(test)]
    pub fn select(&mut self, key: &str) {
        self.selected = Some(key.to_owned());
    }

    #[cfg(test)]
    pub fn selection(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    #[cfg(test)]
    pub fn is_pinned(&self, key: &str) -> bool {
        self.model
            .find(key)
            .is_some_and(|node| self.layout.is_pinned(node))
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        let looks: Vec<Look> = self
            .model
            .nodes
            .iter()
            .map(|node| Look::measure(ui, node))
            .collect();
        let half: Vec<Vec2> = looks.iter().map(|look| look.size / 2.0).collect();
        if self.changed {
            let keys: Vec<String> = self.model.nodes.iter().map(|n| n.key.clone()).collect();
            let links: Vec<(usize, usize)> = self
                .model
                .links
                .iter()
                .map(|link| (link.relation, link.party))
                .collect();
            self.layout.sync(&keys, &half, &links);
            self.layout.settle(FIRST_BUDGET);
            self.changed = false;
        } else {
            self.layout.resize(&half);
        }
        if !self.layout.settled() {
            self.layout.settle(FRAME_BUDGET);
            ui.ctx().request_repaint();
        }
        if self.selected.is_some() && ui.input(|input| input.key_pressed(Key::Escape)) {
            self.selected = None;
        }
        self.toolbar_ui(ui);
        ui.add_space(6.0);
        if ui.available_width() >= WIDE {
            let height = (ui.available_height() - 8.0).max(MIN_CANVAS);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let width = ui.available_width() - INSPECTOR - 12.0;
                self.canvas_ui(ui, vec2(width, height), &looks, &mut action);
                ui.add_space(12.0);
                let column = egui::Layout::top_down(egui::Align::Min);
                ui.allocate_ui_with_layout(vec2(INSPECTOR, height), column, |ui| {
                    ui.set_width(INSPECTOR);
                    self.detail_ui(ui, &mut action);
                });
            });
        } else {
            let height = (ui.available_height() - DETAIL_HEIGHT).max(MIN_CANVAS);
            self.canvas_ui(ui, vec2(ui.available_width(), height), &looks, &mut action);
            ui.add_space(8.0);
            self.detail_ui(ui, &mut action);
        }
        action
    }

    fn toolbar_ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let mut summary = format!(
                "{} · {}",
                plural(self.model.entities(), "entity", "entities"),
                plural(self.model.relations(), "relation", "relations")
            );
            let hidden = self.model.hidden_relations + self.model.hidden_entities;
            if hidden > 0 {
                summary.push_str(&format!(" · {hidden} more in the list, not drawn"));
            }
            widgets::row_detail(ui, &summary);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Rearrange").clicked() {
                    self.layout.rearrange();
                    self.follow = true;
                }
                if ui.button("Fit").clicked() {
                    self.follow = true;
                }
            });
        });
    }

    fn canvas_ui(&mut self, ui: &mut Ui, size: Vec2, looks: &[Look], action: &mut Option<Action>) {
        let palette = theme::palette(ui.ctx());
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().rect(
            rect,
            CornerRadius::same(theme::SECTION_CORNER),
            palette.window,
            Stroke::new(1.0, palette.separator),
            StrokeKind::Inside,
        );
        if self.follow {
            // The whole graph, but never larger than life: a small graph is not blown up.
            let bounds = self.layout.bounds().expand(MARGIN);
            let fit = (bounds.width() / rect.width()).max(bounds.height() / rect.height());
            self.scene = if fit < 1.0 {
                Rect::from_center_size(bounds.center(), rect.size())
            } else {
                bounds
            };
        }
        let mut canvas = ui.new_child(UiBuilder::new().max_rect(rect.shrink(1.0)));
        let selected = self
            .selected
            .as_deref()
            .and_then(|key| self.model.find(key));
        let mut scene_rect = self.scene;
        let response = Scene::new()
            .zoom_range(0.25..=2.0)
            .show(&mut canvas, &mut scene_rect, |ui| {
                draw(
                    ui,
                    &self.model,
                    &mut self.layout,
                    looks,
                    selected,
                    &mut self.selected,
                    action,
                )
            })
            .response;
        self.scene = scene_rect;
        if response.changed() {
            self.follow = false;
        }
        if response.clicked() {
            self.selected = None;
        }
    }

    fn detail_ui(&self, ui: &mut Ui, action: &mut Option<Action>) {
        let Some(node) = self
            .selected
            .as_deref()
            .and_then(|key| self.model.find(key))
            .map(|at| (at, &self.model.nodes[at]))
        else {
            widgets::section(ui, None, |section| {
                section.row(|ui| {
                    widgets::row_title(ui, "Nothing selected");
                    widgets::row_detail(
                        ui,
                        "Select an entity or a relation to see it here. Drag a node to move it, scroll or pinch to zoom, and double-click an entity to add its relations.",
                    );
                });
            });
            return;
        };
        let (at, node) = node;
        let row = node.row.map(|row| &self.model.rows[row]);
        match (node.kind, row) {
            (Kind::Relation, Some(row)) => {
                widgets::section(ui, Some("Relation"), |section| {
                    section.row(|ui| results::relation_ui(ui, row));
                    section.row(|ui| {
                        if ui.button("Show in List").clicked() {
                            *action = Some(Action::ShowInList(row.id.clone()));
                        }
                    });
                });
            }
            (Kind::Literal, Some(row)) => {
                widgets::section(ui, Some("Value"), |section| {
                    section.row(|ui| {
                        widgets::row_title(ui, &node.full);
                        widgets::row_detail(ui, &format!("A value of {}", row.title));
                    });
                });
            }
            _ => {
                widgets::section(ui, Some("Entity"), |section| {
                    section.row(|ui| {
                        widgets::row_with_trailing(
                            ui,
                            |ui| {
                                widgets::row_title(ui, &node.full);
                                let mut detail = node.caption.clone().unwrap_or_default();
                                if !detail.is_empty() {
                                    detail.push_str(" · ");
                                }
                                detail.push_str(&format!(
                                    "{} in this graph",
                                    plural(self.model.degree(at), "relation", "relations")
                                ));
                                widgets::row_detail(ui, &detail);
                            },
                            |ui| {
                                if widgets::primary_button(ui, "Expand").clicked() {
                                    *action = Some(Action::Expand(node.key.clone()));
                                }
                            },
                        );
                    });
                });
            }
        }
    }
}

/// The canvas contents, in scene coordinates.
fn draw(
    ui: &mut Ui,
    model: &Model,
    layout: &mut Layout,
    looks: &[Look],
    selected: Option<usize>,
    selection: &mut Option<String>,
    action: &mut Option<Action>,
) {
    let palette = theme::palette(ui.ctx());
    let to_global = ui.ctx().layer_transform_to_global(ui.layer_id());
    // Every node is a control first, so lines can be drawn under the nodes yet know what is
    // hovered, and a screen reader meets the nodes in the order of the model.
    let mut hovered = None;
    for (i, node) in model.nodes.iter().enumerate() {
        let rect = Rect::from_center_size(layout.position(i), looks[i].size);
        let id = ui.id().with(("graph-node", &node.key));
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let is_selected = selected == Some(i);
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::SelectableLabel, true, is_selected, &node.spoken)
        });
        // AccessKit takes the scene's own coordinates; VoiceOver needs the box on screen.
        if let Some(to_global) = to_global {
            let shown = to_global * rect;
            ui.ctx().accesskit_node_builder(id, |builder| {
                builder.set_bounds(accesskit::Rect {
                    x0: shown.min.x.into(),
                    y0: shown.min.y.into(),
                    x1: shown.max.x.into(),
                    y1: shown.max.y.into(),
                });
            });
        }
        if response.clicked() || response.drag_started() {
            *selection = Some(node.key.clone());
        }
        if response.double_clicked() && node.kind == Kind::Entity {
            *action = Some(Action::Expand(node.key.clone()));
        }
        // A press already counts as a drag; only a move pins a node, so a click never does.
        let moved = response.drag_delta();
        if response.dragged() && moved != Vec2::ZERO {
            layout.drag(i, moved);
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            ui.ctx().request_repaint();
        } else if response.hovered() {
            hovered = Some(i);
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
    }
    let near = selected.map(|node| model.context(node));
    let dimmed = |node: usize| near.as_ref().is_some_and(|near| !near.contains(&node));
    // The lines whose roles are named: those of what is hovered, or else of what is selected.
    let focus = hovered.or(selected);
    let named: BTreeSet<usize> = model
        .links
        .iter()
        .enumerate()
        .filter(|(_, link)| focus.is_some_and(|f| link.relation == f || link.party == f))
        .map(|(at, _)| at)
        .collect();
    let painter = ui.painter();
    for (at, link) in model.links.iter().enumerate() {
        let (a, b) = (link.relation, link.party);
        let row = model.nodes[a].row.map(|row| &model.rows[row]);
        let negated = row.is_some_and(|row| row.negated);
        let start = edge(layout.position(a), looks[a].size / 2.0, layout.position(b));
        let end = edge(layout.position(b), looks[b].size / 2.0, layout.position(a));
        let mut color = if named.contains(&at) || (near.is_some() && !dimmed(a) && !dimmed(b)) {
            palette.crossbar
        } else {
            palette.tertiary
        };
        if dimmed(a) || dimmed(b) {
            color = color.gamma_multiply(DIM);
        }
        let stroke = Stroke::new(if named.contains(&at) { 1.8 } else { 1.2 }, color);
        if negated {
            // A negation is broken wherever it is drawn: its crossbar and its lines.
            painter.extend(Shape::dashed_line(&[start, end], stroke, 6.0, 4.0));
        } else {
            painter.line_segment([start, end], stroke);
        }
    }
    for (i, node) in model.nodes.iter().enumerate() {
        let rect = Rect::from_center_size(layout.position(i), looks[i].size);
        let row = node.row.map(|row| &model.rows[row]);
        let state = State {
            selected: selected == Some(i),
            hovered: hovered == Some(i),
            fade: if dimmed(i) {
                DIM
            } else if node.kind == Kind::Relation && row.is_some_and(|row| row.stale) {
                STALE
            } else {
                1.0
            },
        };
        paint(painter, palette, node, row, &looks[i], rect, state);
    }
    // Role names go over the nodes, in the part of the line that shows between them.
    for &at in &named {
        let link = &model.links[at];
        let (a, b) = (link.relation, link.party);
        let start = edge(layout.position(a), looks[a].size / 2.0, layout.position(b));
        let end = edge(layout.position(b), looks[b].size / 2.0, layout.position(a));
        let text = painter.layout_no_wrap(
            link.role.clone(),
            FontId::new(10.5, FontFamily::Proportional),
            palette.secondary,
        );
        // A name that would cover a node is left to the inspector and the spoken name.
        if (end - start).length() < text.size().x + 12.0 {
            continue;
        }
        let back = Rect::from_center_size(start.lerp(end, 0.5), text.size() + vec2(8.0, 2.0));
        painter.rect(
            back,
            CornerRadius::same(4),
            palette.window,
            Stroke::new(1.0, palette.separator),
            StrokeKind::Inside,
        );
        painter.galley(back.min + vec2(4.0, 1.0), text, palette.secondary);
    }
}

#[derive(Clone, Copy)]
struct State {
    selected: bool,
    hovered: bool,
    fade: f32,
}

fn paint(
    painter: &egui::Painter,
    palette: &Palette,
    node: &model::Node,
    row: Option<&results::RelationRow>,
    look: &Look,
    rect: Rect,
    state: State,
) {
    let fade = |color: Color32| color.gamma_multiply(state.fade);
    let ring = |default: Stroke| {
        if state.selected {
            Stroke::new(2.0, fade(palette.accent_text))
        } else if state.hovered {
            Stroke::new(default.width + 0.5, default.color)
        } else {
            default
        }
    };
    match node.kind {
        Kind::Entity => {
            let fill = if state.selected {
                palette.selection
            } else {
                palette.surface
            };
            painter.rect(
                rect,
                CornerRadius::same(theme::CORNER),
                fade(fill),
                ring(Stroke::new(1.0, fade(palette.separator))),
                StrokeKind::Outside,
            );
            let mut at = rect.min + vec2(10.0, 6.0);
            painter.galley(at, look.main.clone(), fade(palette.text));
            if let Some(caption) = &look.caption {
                at.y += look.main.size().y;
                painter.galley(at, caption.clone(), fade(palette.secondary));
            }
        }
        Kind::Relation => {
            let negated = row.is_some_and(|row| row.negated);
            let hollow = row.is_some_and(|row| row.modality.is_some());
            let proposed = row.is_some_and(|row| row.tone == widgets::Tone::Attention);
            let edge = if proposed {
                palette.warning
            } else {
                palette.crossbar
            };
            let stroke = ring(Stroke::new(1.25, fade(edge)));
            if hollow {
                // Planned or otherwise not plainly asserted: an outline that is not closed.
                let corner = CornerRadius::same(theme::CORNER);
                painter.rect_filled(rect, corner, fade(palette.window));
                let points = [
                    rect.left_top(),
                    rect.right_top(),
                    rect.right_bottom(),
                    rect.left_bottom(),
                    rect.left_top(),
                ];
                painter.extend(Shape::dashed_line(&points, stroke, 4.0, 3.0));
            } else {
                let pill = CornerRadius::same((rect.height() / 2.0).round().min(255.0) as u8);
                painter.rect(
                    rect,
                    pill,
                    fade(palette.surface),
                    stroke,
                    StrokeKind::Outside,
                );
            }
            let y = rect.center().y;
            let left = rect.left() + 11.0;
            let bar = Stroke::new(3.0, fade(palette.crossbar));
            if negated {
                painter.hline(left..=(left + BAR / 2.0 - 2.5), y, bar);
                painter.hline((left + BAR / 2.0 + 2.5)..=(left + BAR), y, bar);
            } else {
                painter.hline(left..=(left + BAR), y, bar);
            }
            let text = Pos2::new(left + BAR + 8.0, y - look.main.size().y / 2.0);
            painter.galley(text, look.main.clone(), fade(palette.text));
        }
        Kind::Literal => {
            painter.rect(
                rect,
                CornerRadius::same(3),
                fade(palette.control_hover),
                ring(Stroke::new(1.0, fade(palette.separator))),
                StrokeKind::Outside,
            );
            painter.galley(
                rect.min + vec2(8.0, 4.0),
                look.main.clone(),
                fade(palette.secondary),
            );
        }
    }
}

/// The answer the graph tests draw.
#[cfg(test)]
pub(crate) use model::tests::answer as fixture;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_leaves_a_box_at_its_edge() {
        let centre = Pos2::new(0.0, 0.0);
        let half = vec2(40.0, 10.0);
        assert_eq!(
            edge(centre, half, Pos2::new(100.0, 0.0)),
            Pos2::new(40.0, 0.0)
        );
        assert_eq!(
            edge(centre, half, Pos2::new(0.0, -50.0)),
            Pos2::new(0.0, -10.0)
        );
        assert_eq!(edge(centre, half, centre), centre);
    }

    #[test]
    fn a_new_answer_starts_a_new_graph_and_an_expansion_keeps_the_selection() {
        let mut graph = Graph::default();
        graph.show(&model::tests::answer());
        graph.select("relation:3");
        assert_eq!(graph.selected_relation(), Some("relation:3"));
        graph.update(&model::tests::answer());
        assert_eq!(graph.selected_relation(), Some("relation:3"));
        graph.select("entity:a");
        assert_eq!(
            graph.selected_relation(),
            None,
            "an entity is not a relation"
        );
        graph.show(&model::tests::answer());
        assert!(graph.selected.is_none());
    }
}
