//! What the Core answered. A relation reads as what it is, two or more parties joined by the
//! crossbar of the icon. Anything else is an outline, and the exact JSON is always one click
//! away, because this is also a tool for inspecting precisely what was returned.
use crate::theme;
use crate::widgets::{self, Tone};
use eframe::egui::{self, vec2, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui};
use huda::error::Error;
use serde_json::Value;

/// Rows beyond this are counted, not drawn: a result can be a megabyte.
const MAX_ROWS: usize = 200;
const MAX_DEPTH: usize = 12;
const MAX_TEXT: usize = 400;

/// One side of a relation.
#[derive(Debug, PartialEq, Eq)]
struct Party {
    role: String,
    name: String,
}

/// A relation as a person reads it.
#[derive(Debug, PartialEq, Eq)]
struct RelationRow {
    title: String,
    key: String,
    parties: Vec<Party>,
    negated: bool,
    /// The modality, unless it is plainly asserted.
    modality: Option<String>,
    status: String,
    tone: Tone,
    assertions: usize,
    stale: bool,
}

fn sentence(text: &str) -> String {
    let words = text.replace('_', " ").to_lowercase();
    let mut letters = words.chars();
    letters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(letters).collect()
    })
}

/// `core:DEPENDS_ON@1` reads as "Depends on". The exact type stays beside it.
fn type_title(relation_type: &str) -> String {
    let name = relation_type.rsplit(':').next().unwrap_or(relation_type);
    sentence(name.split('@').next().unwrap_or(name))
}

fn bounded(text: &str) -> String {
    if text.chars().count() <= MAX_TEXT {
        text.to_owned()
    } else {
        let mut short: String = text.chars().take(MAX_TEXT).collect();
        short.push('…');
        short
    }
}

/// An entity reads as its first label, then its key: an identifier means nothing to a person.
fn entity_name(entities: &[Value], id: &str) -> String {
    let entity = entities.iter().find(|entity| entity["entity_id"] == id);
    entity
        .and_then(|entity| entity["labels"].get(0)?.as_str())
        .or_else(|| entity.and_then(|entity| entity["canonical_key"].as_str()))
        .unwrap_or(id)
        .to_owned()
}

fn literal_text(literal: &Value) -> String {
    let inner = literal.get("value").unwrap_or(literal);
    if let (Some(amount), Some(unit)) = (inner["amount"].as_str(), inner["unit"].as_str()) {
        return format!("{amount} {unit}");
    }
    inner
        .as_str()
        .map_or_else(|| inner.to_string(), str::to_owned)
}

fn party(entities: &[Value], role: &str, value: &Value) -> Party {
    let name = match value["entity_id"].as_str() {
        Some(id) => entity_name(entities, id),
        None => literal_text(&value["literal"]),
    };
    Party {
        role: sentence(role),
        name: bounded(&name),
    }
}

fn publication(assertions: &[Value]) -> (String, Tone) {
    let has = |status: &str| {
        assertions
            .iter()
            .any(|assertion| assertion["publication_status"] == status)
    };
    if has("active") {
        ("Active".to_owned(), Tone::Good)
    } else if has("proposed") {
        ("Proposed".to_owned(), Tone::Attention)
    } else {
        let status = assertions
            .first()
            .and_then(|assertion| assertion["publication_status"].as_str())
            .unwrap_or("no assertion");
        (sentence(status), Tone::Quiet)
    }
}

/// The relations of an answer that has any. An answer of another shape has none, and is shown as
/// an outline rather than guessed at.
fn relation_rows(value: &Value) -> Option<Vec<RelationRow>> {
    let relations = value.get("relations")?.as_array()?;
    let empty = Vec::new();
    let entities = value["entities"].as_array().unwrap_or(&empty);
    let rows = relations.iter().map(|view| {
        let relation = &view["relation"];
        let key = relation["relation_type"].as_str().unwrap_or("").to_owned();
        let mut parties = Vec::new();
        if let Some(roles) = relation["roles"].as_object() {
            for (role, values) in roles {
                for value in values.as_array().unwrap_or(&empty) {
                    parties.push(party(entities, role, value));
                }
            }
        }
        let assertions = view["assertions"].as_array().unwrap_or(&empty);
        let (status, tone) = publication(assertions);
        RelationRow {
            title: type_title(&key),
            key,
            parties,
            negated: relation["polarity"] == "negative",
            modality: relation["modality"]
                .as_str()
                .filter(|modality| *modality != "asserted")
                .map(sentence),
            status,
            tone,
            assertions: assertions.len(),
            stale: assertions
                .iter()
                .any(|assertion| assertion["stale"] == true),
        }
    });
    Some(rows.collect())
}

fn plural(count: usize, one: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {one}s")
    }
}

/// The line under a relation. Whatever would change how it may be read comes first: a negated or
/// planned relation must never pass for a plain fact.
fn relation_detail(row: &RelationRow) -> String {
    let mut parts = Vec::new();
    if row.negated {
        parts.push("Negated".to_owned());
    }
    parts.extend(row.modality.clone());
    if row.stale {
        parts.push("Stale".to_owned());
    }
    parts.push(plural(row.assertions, "assertion"));
    parts.join(" · ")
}

fn party_chip(ui: &mut Ui, party: &Party) {
    let palette = theme::palette(ui.ctx());
    Frame::new()
        .fill(palette.window)
        .stroke(Stroke::new(1.0, palette.separator))
        .corner_radius(CornerRadius::same(theme::CORNER))
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_max_width(260.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(&party.role)
                        .font(theme::subheadline())
                        .color(palette.secondary),
                );
                ui.add(
                    egui::Label::new(RichText::new(&party.name).font(theme::headline())).truncate(),
                );
            });
        });
}

/// The signature of this app: the amber bar of the icon, joining what a relation joins. A negated
/// relation has a broken bar, so that shape says it as well as the words.
fn crossbar(ui: &mut Ui, negated: bool) {
    let palette = theme::palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(26.0, 38.0), Sense::hover());
    let y = rect.center().y;
    let stroke = Stroke::new(3.0, palette.crossbar);
    if negated {
        let gap = 4.0;
        ui.painter()
            .hline(rect.left()..=(rect.center().x - gap), y, stroke);
        ui.painter()
            .hline((rect.center().x + gap)..=rect.right(), y, stroke);
    } else {
        ui.painter().hline(rect.left()..=rect.right(), y, stroke);
    }
}

/// What a relation is called aloud. The crossbar shows a negation at a glance, and a screen reader
/// reads a row in order, so the words that change its meaning come before its parties.
fn spoken_title(row: &RelationRow) -> String {
    let mut words = Vec::new();
    if row.negated {
        words.push("Not".to_owned());
    }
    words.extend(row.modality.clone());
    words.push(row.title.clone());
    words.join(", ")
}

fn relation_ui(ui: &mut Ui, row: &RelationRow) {
    let palette = theme::palette(ui.ctx());
    widgets::row_with_trailing(
        ui,
        |ui| {
            let title =
                ui.label(RichText::new(&row.title).font(egui::FontId::new(13.0, theme::medium())));
            let spoken = spoken_title(row);
            title
                .ctx
                .accesskit_node_builder(title.id, |node| node.set_value(spoken));
            ui.label(
                RichText::new(&row.key)
                    .monospace()
                    .size(11.0)
                    .color(palette.secondary),
            );
        },
        |ui| widgets::status(ui, row.tone, &row.status),
    );
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (index, party) in row.parties.iter().enumerate() {
            if index > 0 {
                crossbar(ui, row.negated);
            }
            party_chip(ui, party);
        }
    });
    ui.add_space(2.0);
    widgets::row_detail(ui, &relation_detail(row));
}

fn scalar_text(value: &Value) -> String {
    value.as_str().map_or_else(|| value.to_string(), bounded)
}

fn outline(ui: &mut Ui, key: &str, value: &Value, depth: usize) {
    let palette = theme::palette(ui.ctx());
    let children: Vec<(String, &Value)> = match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| (key.clone(), value))
            .collect(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, value)| (index.to_string(), value))
            .collect(),
        _ => {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(key).color(palette.secondary));
                ui.add(
                    egui::Label::new(RichText::new(scalar_text(value)).monospace())
                        .selectable(true),
                );
            });
            return;
        }
    };
    if depth >= MAX_DEPTH {
        ui.label(RichText::new(format!("{key}: open the JSON view for the rest")).weak());
        return;
    }
    egui::CollapsingHeader::new(format!("{key}  ({})", children.len()))
        .id_salt(ui.id().with(key).with(depth))
        .default_open(depth < 2)
        .show(ui, |ui| {
            for (key, value) in children.iter().take(MAX_ROWS) {
                outline(ui, key, value, depth + 1);
            }
            if children.len() > MAX_ROWS {
                ui.label(format!(
                    "{} more; the JSON view holds them",
                    children.len() - MAX_ROWS
                ));
            }
        });
}

/// The Core's latest answer.
pub struct Results {
    value: Option<Value>,
    json: String,
    error: Option<String>,
    message: String,
    caution: bool,
    show_json: bool,
}

impl Results {
    pub fn new(message: String) -> Self {
        Self {
            value: None,
            json: String::new(),
            error: None,
            message,
            caution: false,
            show_json: false,
        }
    }

    /// A short statement from the app itself, which is not an answer from the Core.
    pub fn message(&mut self, message: impl Into<String>) {
        self.value = None;
        self.json.clear();
        self.error = None;
        self.message = message.into();
        self.caution = false;
    }

    /// What the results area says, for a test.
    #[cfg(test)]
    pub fn shown(&self) -> String {
        format!("{}{}", self.error.clone().unwrap_or_default(), self.message)
    }

    /// A statement from the app that asks for something before anything else can happen.
    pub fn attention(&mut self, message: impl Into<String>) {
        self.message(message);
        self.caution = true;
    }

    pub fn set(&mut self, result: Result<Value, Error>) {
        self.message.clear();
        self.caution = false;
        match result {
            Ok(value) => {
                self.json = serde_json::to_string_pretty(&value)
                    .unwrap_or_else(|_| "Response could not be displayed".into());
                self.value = Some(value);
                self.error = None;
            }
            Err(error) => {
                self.value = None;
                self.json.clear();
                self.error = Some(format!("{}: {}", error.code, error.message));
            }
        }
    }

    fn relations_ui(ui: &mut Ui, value: &Value, rows: &[RelationRow]) {
        if rows.is_empty() {
            widgets::section(ui, None, |section| {
                section.row(|ui| {
                    widgets::row_title(ui, "No relations yet");
                    widgets::row_detail(
                        ui,
                        "Assert one in the Editor, or admit a source. Nothing here was hidden from you.",
                    );
                });
            });
            return;
        }
        widgets::section(ui, Some(&plural(rows.len(), "relation")), |section| {
            for row in rows.iter().take(MAX_ROWS) {
                section.row(|ui| relation_ui(ui, row));
            }
        });
        if rows.len() > MAX_ROWS {
            ui.label(format!(
                "{} more; the JSON view holds them",
                rows.len() - MAX_ROWS
            ));
        }
        if value["truncated"] == true {
            widgets::notice(
                ui,
                Tone::Attention,
                "This is part of the result: the query reached its bound.",
            );
        }
    }

    fn answer_ui(ui: &mut Ui, value: &Value) {
        if let Some(answer) = value["answer"].as_str() {
            widgets::section(ui, Some("Answer"), |section| {
                section.row(|ui| {
                    ui.add(egui::Label::new(answer).selectable(true));
                });
            });
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        if let Some(error) = &self.error {
            widgets::notice(ui, Tone::Problem, error);
        }
        if !self.message.is_empty() {
            let tone = if self.caution {
                Tone::Attention
            } else {
                Tone::Quiet
            };
            widgets::notice(ui, tone, &self.message);
        }
        let Some(value) = &self.value else { return };
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.show_json, false, "Readable");
            ui.selectable_value(&mut self.show_json, true, "JSON");
            if ui.button("Copy JSON").clicked() {
                ui.ctx().copy_text(self.json.clone());
            }
        });
        ui.add_space(4.0);
        egui::ScrollArea::both()
            .id_salt("result")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                widgets::leave_room_for_scrollbar(ui);
                if self.show_json {
                    let mut json = self.json.as_str();
                    let text = ui.add(
                        egui::TextEdit::multiline(&mut json)
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    );
                    widgets::labelled(&text, "Result as JSON");
                } else if let Some(rows) = relation_rows(value) {
                    Self::relations_ui(ui, value, &rows);
                } else {
                    Self::answer_ui(ui, value);
                    outline(ui, "Result", value, 0);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use huda::relation::{query::EntityView, RelationView};
    use serde_json::json;

    /// The real serialized view, so this reads the shape the Core actually sends.
    fn explore(polarity: &str, modality: &str, status: &str, stale: bool) -> Value {
        let assertion = json!({
            "assertion_id": "a1",
            "assertion_version_id": "v1",
            "version": 1,
            "publication_status": status,
            "epistemic_kind": "asserted",
            "origin": {
                "kind": "manual",
                "actor": "owner",
                "operation_id": "o1",
                "source_key": null,
                "source_revision": null,
                "source_generation": null,
                "extraction_scope_key": null,
                "extractor_digest": null,
            },
            "scope": "local",
            "tx_tick": 1,
            "stale": stale,
            "evidence_refs": [],
        });
        let view = json!({
            "relation_id": "relation:1",
            "relation_version_id": "version:1",
            "relation": {
                "relation_type": "core:DEPENDS_ON@1",
                "roles": {
                    "dependent": [{"kind": "entity", "entity_id": "entity:a"}],
                    "dependency": [{"kind": "entity", "entity_id": "entity:b"}],
                },
                "qualifiers": {},
                "polarity": polarity,
                "modality": modality,
            },
            "assertions": [assertion],
        });
        json!({
            "entities": [
                {"entity_id": "entity:a", "namespace": "core", "kind": "project",
                 "canonical_key": "project-a", "labels": ["Project A"]},
                {"entity_id": "entity:b", "namespace": "core", "kind": "project",
                 "canonical_key": "library-b", "labels": []},
            ],
            "relations": [view],
            "truncated": false,
            "snapshot_tick": 1,
        })
    }

    fn one(value: &Value) -> RelationRow {
        relation_rows(value).unwrap().remove(0)
    }

    #[test]
    fn a_relation_reads_with_names_not_identifiers() {
        let row = one(&explore("positive", "asserted", "active", false));
        assert_eq!(row.title, "Depends on");
        assert_eq!(row.key, "core:DEPENDS_ON@1");
        let names: Vec<_> = row
            .parties
            .iter()
            .map(|party| party.name.as_str())
            .collect();
        assert_eq!(names, ["library-b", "Project A"]);
        assert_eq!(row.parties[1].role, "Dependent");
        assert_eq!((row.status.as_str(), row.tone), ("Active", Tone::Good));
    }

    #[test]
    fn a_negated_or_planned_relation_never_passes_for_a_plain_fact() {
        let row = one(&explore("negative", "planned", "active", false));
        assert!(row.negated);
        assert_eq!(relation_detail(&row), "Negated · Planned · 1 assertion");
        let plain = one(&explore("positive", "asserted", "active", false));
        assert_eq!(relation_detail(&plain), "1 assertion");
    }

    #[test]
    fn publication_and_staleness_come_from_the_assertions() {
        let proposed = one(&explore("positive", "asserted", "proposed", true));
        assert_eq!(proposed.tone, Tone::Attention);
        assert!(relation_detail(&proposed).contains("Stale"));
        let withdrawn = one(&explore("positive", "asserted", "withdrawn", false));
        assert_eq!(
            (withdrawn.status.as_str(), withdrawn.tone),
            ("Withdrawn", Tone::Quiet)
        );
    }

    #[test]
    fn a_literal_party_shows_its_value_and_an_unknown_entity_its_identifier() {
        let mut value = explore("positive", "asserted", "active", false);
        value["relation"] = Value::Null;
        value["relations"][0]["relation"]["roles"] = json!({
            "subject": [{"kind": "entity", "entity_id": "entity:unknown"}],
            "object": [{"kind": "literal", "literal": {"type": "string", "value": "2026"}}],
        });
        let row = one(&value);
        let names: Vec<_> = row
            .parties
            .iter()
            .map(|party| party.name.as_str())
            .collect();
        assert_eq!(names, ["2026", "entity:unknown"]);
    }

    #[test]
    fn every_kind_of_literal_reads_as_its_value() {
        use huda::relation::Literal;
        for (literal, text) in [
            (Literal::String("Berlin".into()), "Berlin"),
            (Literal::Integer(7), "7"),
            (Literal::Boolean(true), "true"),
            (Literal::Decimal("1.50".into()), "1.50"),
            (
                Literal::Quantity {
                    amount: "3".into(),
                    unit: "kg".into(),
                },
                "3 kg",
            ),
        ] {
            assert_eq!(literal_text(&serde_json::to_value(literal).unwrap()), text);
        }
    }

    #[test]
    fn an_answer_of_another_shape_is_not_guessed_at() {
        assert!(relation_rows(&json!({"status": "ok"})).is_none());
        assert!(relation_rows(&json!([1, 2])).is_none());
    }

    #[test]
    fn the_fixture_is_what_the_core_really_sends() {
        // These types refuse unknown fields, so this fails if the fixture drifts from them, and
        // the parser then reads the real serialization rather than the fixture.
        let fixture = explore("negative", "planned", "proposed", true);
        let relation: RelationView =
            serde_json::from_value(fixture["relations"][0].clone()).unwrap();
        let entity: EntityView = serde_json::from_value(fixture["entities"][0].clone()).unwrap();
        let mut real = fixture.clone();
        real["relations"][0] = serde_json::to_value(&relation).unwrap();
        real["entities"][0] = serde_json::to_value(&entity).unwrap();
        assert_eq!(one(&real), one(&fixture));
    }

    #[test]
    fn a_failure_replaces_the_last_answer() {
        let mut results = Results::new(String::new());
        results.set(Ok(json!({"a": 1})));
        assert!(results.value.is_some());
        results.set(Err(Error::new("AUTH_REQUIRED", "Unlock it")));
        assert!(results.value.is_none());
        assert_eq!(results.error.as_deref(), Some("AUTH_REQUIRED: Unlock it"));
    }

    #[test]
    fn every_view_draws_without_a_window() {
        let context = egui::Context::default();
        crate::theme::apply(&context);
        for value in [
            explore("negative", "planned", "proposed", true),
            json!({"relations": [], "entities": [], "truncated": true}),
            json!({"answer": "Text", "status": "answered", "nested": {"a": [1, 2]}}),
            json!([1, {"deep": true}]),
        ] {
            let mut results = Results::new(String::new());
            results.set(Ok(value));
            let mut output = context.run_ui(egui::RawInput::default(), |ui| results.ui(ui));
            output.textures_delta.clear();
            assert!(!output.shapes.is_empty());
        }
    }
}
