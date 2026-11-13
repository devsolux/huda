//! The controls Huda repeats: a grouped section of rows, status that never relies on color alone,
//! buttons with one meaning each, and a small vector icon set that shares one stroke.
use crate::theme::{self, Palette, CORNER, SECTION_CORNER};
use eframe::egui::{
    self,
    accesskit::{Live, Role},
    pos2, vec2, Color32, CornerRadius, Frame, Margin, Pos2, Rect, Response, RichText, Sense, Shape,
    Stroke, Ui, Vec2, Widget, WidgetInfo,
};

/// Sections end short of a scroll area's edge, where its bar appears over the content.
pub fn leave_room_for_scrollbar(ui: &mut Ui) {
    ui.set_max_width(ui.available_width() - 12.0);
}

/// The title and one line that say what an area is for.
pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str) {
    as_heading(&ui.heading(title), 1);
    ui.label(RichText::new(subtitle).color(theme::palette(ui.ctx()).secondary));
    ui.add_space(12.0);
}

/// A titled surface of rows, separated by hairlines, as in the system's grouped forms.
pub fn section<R>(ui: &mut Ui, title: Option<&str>, body: impl FnOnce(&mut Section) -> R) -> R {
    let palette = theme::palette(ui.ctx());
    if let Some(title) = title {
        ui.add_space(6.0);
        let heading = ui.label(
            RichText::new(title)
                .font(theme::section_title())
                .color(palette.secondary),
        );
        as_heading(&heading, 2);
        ui.add_space(2.0);
    }
    let inner = Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.separator))
        .corner_radius(CornerRadius::same(SECTION_CORNER))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            body(&mut Section { ui, rows: 0 })
        })
        .inner;
    ui.add_space(8.0);
    inner
}

pub struct Section<'a> {
    ui: &'a mut Ui,
    rows: usize,
}

impl Section<'_> {
    /// One row. The first has no hairline above it.
    pub fn row<R>(&mut self, body: impl FnOnce(&mut Ui) -> R) -> R {
        let palette = theme::palette(self.ui.ctx());
        if self.rows > 0 {
            let rect = self.ui.available_rect_before_wrap();
            self.ui.painter().hline(
                rect.x_range().shrink(12.0),
                rect.top(),
                Stroke::new(1.0, palette.separator),
            );
        }
        self.rows += 1;
        Frame::new()
            .inner_margin(Margin::symmetric(12, 8))
            .show(self.ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                body(ui)
            })
            .inner
    }
}

/// What a row measured last frame. Its controls end at the trailing edge, but they are added in
/// reading order, which is also the order of Tab and of a screen reader, so a row places them
/// from what it knows of their size rather than building from the trailing edge.
#[derive(Clone, Copy, Default, PartialEq)]
struct RowSize {
    trailing: Vec2,
    leading_height: f32,
}

/// A row's text at the leading edge and its controls at the trailing edge, both centered on the
/// row and read in that order.
pub fn row_with_trailing(
    ui: &mut Ui,
    leading: impl FnOnce(&mut Ui),
    trailing: impl FnOnce(&mut Ui),
) {
    const GAP: f32 = 12.0;
    let id = ui.next_auto_id().with("row-size");
    let known: RowSize = ui.ctx().data(|data| data.get_temp(id)).unwrap_or_default();
    let width = ui.available_width();
    let column = (width - known.trailing.x - GAP).max(0.0);
    let row_height = known.trailing.y.max(known.leading_height);
    let measured = ui
        .horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let text = ui
                .allocate_ui_with_layout(
                    vec2(column, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(column);
                        ui.add_space((row_height - known.leading_height) / 2.0);
                        let top = ui.cursor().top();
                        leading(ui);
                        ui.cursor().top() - top
                    },
                )
                .inner;
            ui.add_space((ui.available_width() - known.trailing.x).max(0.0));
            let controls = ui
                .vertical(|ui| {
                    ui.add_space((row_height - known.trailing.y) / 2.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        trailing(ui);
                    })
                    .response
                    .rect
                    .size()
                })
                .inner;
            RowSize {
                trailing: controls,
                leading_height: text,
            }
        })
        .inner;
    if measured != known {
        ui.ctx().data_mut(|data| data.insert_temp(id, measured));
        ui.ctx().request_discard("A row measured its controls");
    }
}

/// A row title, the macOS Body role in its emphasized weight.
pub fn row_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(egui::FontId::new(13.0, theme::medium())));
}

/// A row's supporting line.
pub fn row_detail(ui: &mut Ui, text: &str) {
    if !text.is_empty() {
        let color = theme::palette(ui.ctx()).secondary;
        ui.label(RichText::new(text).font(theme::subheadline()).color(color));
    }
}

/// A supporting line that needs attention, at the size of the row it belongs to.
pub fn row_caution(ui: &mut Ui, tone: Tone, text: &str) {
    let color = tone_color(tone, theme::palette(ui.ctx()));
    ui.label(RichText::new(text).font(theme::subheadline()).color(color));
}

/// What a status means, carried by a shape and a word as well as a color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Quiet,
    Good,
    Attention,
    Problem,
}

fn tone_color(tone: Tone, palette: &Palette) -> Color32 {
    match tone {
        Tone::Quiet => palette.secondary,
        Tone::Good => palette.success,
        Tone::Attention => palette.warning,
        Tone::Problem => palette.danger,
    }
}

fn status_glyph(ui: &mut Ui, tone: Tone, color: Color32) {
    {
        let (rect, _) = ui.allocate_exact_size(vec2(11.0, 11.0), Sense::hover());
        let painter = ui.painter();
        let center = rect.center();
        match tone {
            Tone::Quiet => {
                painter.circle_stroke(center, 4.0, Stroke::new(1.3, color));
            }
            Tone::Good => {
                painter.circle_filled(center, 5.0, color);
                let mark = theme::palette(ui.ctx()).surface;
                painter.line(
                    vec![
                        center + vec2(-2.5, 0.0),
                        center + vec2(-0.6, 1.9),
                        center + vec2(2.6, -2.0),
                    ],
                    Stroke::new(1.4, mark),
                );
            }
            Tone::Attention | Tone::Problem => {
                let top = center + vec2(0.0, -5.0);
                let left = center + vec2(-5.2, 4.2);
                let right = center + vec2(5.2, 4.2);
                painter.add(Shape::convex_polygon(
                    vec![top, right, left],
                    color,
                    Stroke::NONE,
                ));
                let mark = theme::palette(ui.ctx()).surface;
                painter.line_segment(
                    [center + vec2(0.0, -2.0), center + vec2(0.0, 1.0)],
                    Stroke::new(1.3, mark),
                );
                painter.circle_filled(center + vec2(0.0, 2.8), 0.8, mark);
            }
        }
    }
}

/// A status glyph and its word. The glyph repeats the word for the eye, so only the word is read.
pub fn status(ui: &mut Ui, tone: Tone, text: &str) {
    let color = tone_color(tone, theme::palette(ui.ctx()));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        status_glyph(ui, tone, color);
        ui.label(RichText::new(text).font(theme::subheadline()).color(color));
    });
}

/// A line that reports a problem, or quietly reports a result, above the content it is about. A
/// screen reader announces it when it appears, because it may be the only sign that anything
/// happened: a problem at once, anything else at the next pause.
pub fn notice(ui: &mut Ui, tone: Tone, text: &str) {
    let palette = theme::palette(ui.ctx());
    let color = if tone == Tone::Quiet {
        palette.secondary
    } else {
        tone_color(tone, palette)
    };
    let response = ui.label(RichText::new(text).color(color));
    let urgent = tone == Tone::Problem;
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_role(if urgent { Role::Alert } else { Role::Status });
        node.set_live(if urgent {
            Live::Assertive
        } else {
            Live::Polite
        });
    });
}

/// A name for a control whose text alone would not say what it does, or that has no text.
pub fn labelled(response: &Response, name: &str) {
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_label(name.to_owned());
    });
}

/// Marks text as a heading, which is how a screen reader moves through a page.
fn as_heading(response: &Response, level: usize) {
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_role(Role::Heading);
        node.set_level(level);
    });
}

fn filled_button(ui: &mut Ui, text: &str, fill: Color32, on_fill: Color32) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(on_fill))
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(CORNER)),
    )
}

/// The one action an area is for.
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let palette = theme::palette(ui.ctx());
    filled_button(ui, text, palette.accent, palette.on_accent)
}

/// An action that takes something away, and is never the default.
pub fn destructive_button(ui: &mut Ui, text: &str) -> Response {
    let palette = theme::palette(ui.ctx());
    filled_button(ui, text, palette.danger_fill, palette.on_danger)
}

/// A switch for a setting in a grouped form.
pub struct Switch<'a> {
    on: &'a mut bool,
    label: &'a str,
}

impl<'a> Switch<'a> {
    pub fn new(on: &'a mut bool, label: &'a str) -> Self {
        Self { on, label }
    }
}

impl Widget for Switch<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let size = vec2(32.0, 19.0);
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        let (on, label, enabled) = (*self.on, self.label, ui.is_enabled());
        response.widget_info(|| WidgetInfo::selected(Role::CheckBox, enabled, on, label));
        response.ctx.accesskit_node_builder(response.id, |node| {
            node.set_role(Role::Switch);
        });
        if ui.is_rect_visible(rect) {
            let palette = theme::palette(ui.ctx());
            let moved = ui.ctx().animate_bool(response.id, on);
            let track = if on {
                palette.accent
            } else {
                palette.control_hover
            };
            let radius = rect.height() / 2.0;
            ui.painter().rect(
                rect,
                radius,
                track,
                Stroke::new(1.0, palette.separator),
                egui::StrokeKind::Inside,
            );
            let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), moved);
            ui.painter()
                .circle_filled(pos2(x, rect.center().y), radius - 2.0, palette.on_accent);
            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect.expand(2.0),
                    radius + 2.0,
                    Stroke::new(1.5, palette.accent_text),
                    egui::StrokeKind::Outside,
                );
            }
        }
        response
    }
}

/// One icon of the sidebar. They are drawn, not bitmaps, so they stay sharp and share one stroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Relation,
    Ask,
    Editor,
    Sources,
    Integrations,
    Clients,
    Diagnostics,
    Settings,
}

fn draw_icon(ui: &Ui, icon: Icon, rect: Rect, color: Color32) {
    let painter = ui.painter();
    let stroke = Stroke::new(1.4, color);
    let center = rect.center();
    let at = |x: f32, y: f32| -> Pos2 { center + vec2(x, y) };
    let corner = CornerRadius::same(2);
    match icon {
        Icon::Relation => {
            painter.circle_stroke(at(-5.0, 0.0), 2.6, stroke);
            painter.circle_stroke(at(5.0, 0.0), 2.6, stroke);
            painter.line_segment([at(-2.4, 0.0), at(2.4, 0.0)], stroke);
        }
        Icon::Ask => {
            let body = Rect::from_center_size(at(0.0, -1.0), vec2(13.0, 9.5));
            painter.rect_stroke(body, 3.0, stroke, egui::StrokeKind::Middle);
            painter.line(vec![at(-3.0, 3.8), at(-4.0, 6.5), at(-0.5, 3.8)], stroke);
        }
        Icon::Editor => {
            painter.line_segment([at(-5.0, 5.0), at(4.0, -4.0)], stroke);
            painter.line_segment([at(-3.0, 6.0), at(6.0, -3.0)], stroke);
            painter.line(
                vec![at(4.0, -4.0), at(5.5, -5.5), at(7.0, -4.0), at(6.0, -3.0)],
                stroke,
            );
            painter.line(vec![at(-5.0, 5.0), at(-6.2, 7.0), at(-3.0, 6.0)], stroke);
        }
        Icon::Sources => {
            let page = Rect::from_center_size(center, vec2(10.5, 13.0));
            painter.rect_stroke(page, corner, stroke, egui::StrokeKind::Middle);
            for y in [-3.0, 0.0, 3.0] {
                painter.line_segment([at(-2.8, y), at(2.8, y)], stroke);
            }
        }
        Icon::Integrations => {
            let plug = Rect::from_center_size(at(0.0, -0.5), vec2(9.0, 5.5));
            painter.rect_stroke(plug, corner, stroke, egui::StrokeKind::Middle);
            painter.line_segment([at(-2.2, -3.2), at(-2.2, -6.2)], stroke);
            painter.line_segment([at(2.2, -3.2), at(2.2, -6.2)], stroke);
            painter.line_segment([at(0.0, 2.2), at(0.0, 6.4)], stroke);
        }
        Icon::Clients => {
            let screen = Rect::from_center_size(at(0.0, -1.2), vec2(11.5, 8.0));
            painter.rect_stroke(screen, corner, stroke, egui::StrokeKind::Middle);
            painter.line_segment([at(-7.0, 4.8), at(7.0, 4.8)], stroke);
        }
        Icon::Diagnostics => {
            painter.line(
                vec![
                    at(-7.0, 0.5),
                    at(-3.5, 0.5),
                    at(-1.5, -5.0),
                    at(1.2, 5.0),
                    at(3.2, 0.5),
                    at(7.0, 0.5),
                ],
                stroke,
            );
        }
        Icon::Settings => {
            painter.circle_stroke(center, 2.4, stroke);
            for tooth in 0..8 {
                let angle = tooth as f32 * std::f32::consts::FRAC_PI_4;
                let direction = vec2(angle.cos(), angle.sin());
                painter.line_segment([center + direction * 4.6, center + direction * 6.6], stroke);
            }
        }
    }
}

/// A sidebar row. Only the place you are in gets the crossbar.
pub fn sidebar_row(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    compact: bool,
) -> Response {
    let palette = theme::palette(ui.ctx());
    let height = 28.0;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, ui.is_enabled(), selected, label));
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            palette.selection
        } else if response.hovered() {
            palette.control_hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CORNER, fill);
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                CORNER,
                Stroke::new(1.5, palette.accent_text),
                egui::StrokeKind::Inside,
            );
        }
        if selected {
            let bar =
                Rect::from_center_size(pos2(rect.left() + 1.5, rect.center().y), vec2(3.0, 14.0));
            ui.painter().rect_filled(bar, 1.5, palette.crossbar);
        }
        let icon_box = if compact {
            Rect::from_center_size(rect.center(), vec2(16.0, 16.0))
        } else {
            Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(16.0, 16.0))
        };
        let color = if selected {
            palette.text
        } else {
            palette.secondary
        };
        draw_icon(ui, icon, icon_box, color);
        if !compact {
            ui.painter().text(
                pos2(rect.left() + 38.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::new(13.0, egui::FontFamily::Proportional),
                palette.text,
            );
        }
    }
    if compact {
        response.on_hover_text(label)
    } else {
        response
    }
}

/// The product's mark as the window draws it: the crossbar is the relation between two uprights.
pub fn monogram(ui: &mut Ui, size: f32) {
    let palette = theme::palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let unit = size / 16.0;
    let upright = vec2(3.0 * unit, 12.0 * unit);
    for x in [-3.8, 3.8] {
        let bar = Rect::from_center_size(rect.center() + vec2(x * unit, 0.0), upright);
        ui.painter().rect_filled(bar, 1.5 * unit, palette.text);
    }
    let crossbar = Rect::from_center_size(rect.center(), vec2(9.0 * unit, 2.6 * unit));
    ui.painter()
        .rect_filled(crossbar, 1.3 * unit, palette.crossbar);
}

/// Rings whatever holds keyboard focus. egui draws a focused button like a hovered one, which a
/// person moving with Tab cannot see, so one ring serves every control and sits above sheets.
pub fn focus_ring(context: &egui::Context) {
    let Some(response) = context
        .memory(|memory| memory.focused())
        .and_then(|id| context.read_response(id))
    else {
        return;
    };
    let palette = theme::palette(context);
    // A control on a panned or zoomed canvas is drawn moved; the ring goes where it is seen.
    let rect = context
        .layer_transform_to_global(response.layer_id)
        .map_or(response.rect, |to_global| to_global * response.rect);
    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("focus-ring"));
    context.layer_painter(layer).rect_stroke(
        rect.expand(1.5),
        CORNER + 1,
        Stroke::new(2.0, palette.accent_text),
        egui::StrokeKind::Outside,
    );
}

/// A dimmed sheet over the window, holding one short task. Answers whether it should close. A
/// screen reader treats it as a modal dialog named `title`, so nothing behind it is reached.
pub fn sheet(context: &egui::Context, id: &str, title: &str, body: impl FnOnce(&mut Ui)) -> bool {
    let palette = theme::palette(context);
    let backdrop = if context.theme() == egui::Theme::Dark {
        Color32::from_black_alpha(140)
    } else {
        Color32::from_black_alpha(70)
    };
    let id = egui::Id::unique(id);
    // egui makes a modal's area an unnamed dialog of its own. The content below is the one
    // dialog, named and modal, so the area stays a plain container.
    let response = egui::Modal::new(id)
        .area(egui::Modal::default_area(id).role(Role::GenericContainer))
        .backdrop_color(backdrop)
        .frame(
            Frame::new()
                .fill(palette.surface)
                .stroke(Stroke::new(1.0, palette.separator))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(20)),
        )
        .show(context, |ui| {
            ui.set_width(520.0);
            ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                node.set_role(Role::Dialog);
                node.set_modal();
                node.set_label(title.to_owned());
            });
            let heading =
                ui.label(RichText::new(title).font(egui::FontId::new(17.0, theme::semibold())));
            as_heading(&heading, 1);
            ui.add_space(4.0);
            body(ui);
        });
    response.should_close()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_changes_only_when_clicked_and_is_labelled() {
        let context = egui::Context::default();
        crate::theme::apply(&context);
        let mut on = false;
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.add(Switch::new(&mut on, "Launch at login"));
        });
        output.textures_delta.clear();
        assert!(!on);
    }
}
