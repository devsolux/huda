//! What the graph shows. Every entity appears once. Every relation is a node of its own, joined to
//! each of its parties by a line named for the role, because a relation can join any number of
//! parties and a line can only join two. A literal belongs to the one relation that holds it, so
//! two relations that both say "2026" do not look connected through it.
use crate::results::{self, RelationRow};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// Past this many nodes a graph stops being readable. The rest is counted, and the list holds it.
pub const MAX_NODES: usize = 300;
/// Longer names are shortened on the canvas; the spoken name and the detail keep them whole.
const MAX_LABEL: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Entity,
    Relation,
    Literal,
}

#[derive(Debug)]
pub struct Node {
    /// What the node is, the same in every answer: an entity or relation ID, or for a literal its
    /// relation and place. Positions and the selection follow it.
    pub key: String,
    pub kind: Kind,
    /// The name in full, for the detail under the canvas.
    pub full: String,
    /// The name as drawn, shortened if long.
    pub label: String,
    /// The entity's kind, under its name.
    pub caption: Option<String>,
    /// What a screen reader says: the whole name, and for a relation every party.
    pub spoken: String,
    /// The relation this node is, or holds as a literal, as an index into `rows`.
    pub row: Option<usize>,
}

/// One party of one relation.
#[derive(Debug)]
pub struct Link {
    pub relation: usize,
    pub party: usize,
    pub role: String,
}

#[derive(Debug, Default)]
pub struct Model {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    pub rows: Vec<RelationRow>,
    /// Relations and entities left out at the node bound.
    pub hidden_relations: usize,
    pub hidden_entities: usize,
    index: HashMap<String, usize>,
}

fn short(text: &str) -> String {
    if text.chars().count() <= MAX_LABEL {
        text.to_owned()
    } else {
        let mut short: String = text.chars().take(MAX_LABEL - 1).collect();
        short.push('…');
        short
    }
}

/// What a relation node says: the words that change how it may be read come first, as in the
/// list, and a status other than active is a word too, never only a color.
fn relation_label(row: &RelationRow) -> String {
    let mut words = Vec::new();
    if row.negated {
        words.push("Not".to_owned());
    }
    words.extend(row.modality.clone());
    words.push(row.title.clone());
    if row.status != "Active" {
        words.push(row.status.clone());
    }
    if row.stale {
        words.push("Stale".to_owned());
    }
    words.join(" · ")
}

fn relation_spoken(row: &RelationRow) -> String {
    let parties: Vec<String> = row
        .parties
        .iter()
        .map(|party| format!("{} as {}", party.name, party.role.to_lowercase()))
        .collect();
    let mut spoken = format!(
        "{}: {}. {}",
        results::spoken_title(row),
        parties.join(", "),
        row.status
    );
    if row.stale {
        spoken.push_str(", stale");
    }
    spoken
}

impl Model {
    /// The graph of an answer that has relations; an answer of another shape has none.
    pub fn of(value: &Value) -> Option<Self> {
        let rows = results::relation_rows(value)?;
        let empty = Vec::new();
        let entities = value["entities"].as_array().unwrap_or(&empty);
        let mut model = Self::default();
        for (place, row) in rows.into_iter().enumerate() {
            let new: BTreeSet<&str> = row
                .parties
                .iter()
                .filter_map(|party| party.entity.as_deref())
                .filter(|id| !model.index.contains_key(*id))
                .collect();
            let literals = row.parties.iter().filter(|p| p.entity.is_none()).count();
            if model.nodes.len() + 1 + new.len() + literals > MAX_NODES {
                model.hidden_relations += 1;
                continue;
            }
            let at = model.rows.len();
            let key = if row.id.is_empty() {
                format!("relation#{place}")
            } else {
                row.id.clone()
            };
            let label = relation_label(&row);
            let relation = model.add(Node {
                key: key.clone(),
                kind: Kind::Relation,
                label: short(&label),
                full: label,
                caption: None,
                spoken: relation_spoken(&row),
                row: Some(at),
            });
            for (index, party) in row.parties.iter().enumerate() {
                let node = match &party.entity {
                    Some(id) => model.entity(entities, id),
                    None => model.add(Node {
                        key: format!("{key}#{index}"),
                        kind: Kind::Literal,
                        label: short(&party.name),
                        full: party.name.clone(),
                        caption: None,
                        spoken: format!("Value {}", party.name),
                        row: Some(at),
                    }),
                };
                model.links.push(Link {
                    relation,
                    party: node,
                    role: party.role.clone(),
                });
            }
            model.rows.push(row);
        }
        // An entity that matched a query keeps its place even when none of its relations did.
        for id in entities
            .iter()
            .filter_map(|entity| entity["entity_id"].as_str())
        {
            if model.index.contains_key(id) {
                continue;
            }
            if model.nodes.len() < MAX_NODES {
                model.entity(entities, id);
            } else {
                model.hidden_entities += 1;
            }
        }
        model.name_entities();
        Some(model)
    }

    fn add(&mut self, node: Node) -> usize {
        let at = self.nodes.len();
        self.index.insert(node.key.clone(), at);
        self.nodes.push(node);
        at
    }

    fn entity(&mut self, entities: &[Value], id: &str) -> usize {
        if let Some(&at) = self.index.get(id) {
            return at;
        }
        let name = results::entity_name(entities, id);
        let caption = entities
            .iter()
            .find(|entity| entity["entity_id"] == id)
            .and_then(|entity| entity["kind"].as_str())
            .map(results::sentence);
        self.add(Node {
            key: id.to_owned(),
            kind: Kind::Entity,
            label: short(&name),
            full: results::bounded(&name),
            caption,
            spoken: results::bounded(&name),
            row: None,
        })
    }

    /// An entity is read with what it is and how much of it this graph shows.
    fn name_entities(&mut self) {
        let mut counts = vec![0; self.nodes.len()];
        for link in &self.links {
            counts[link.party] += 1;
        }
        for (node, count) in self.nodes.iter_mut().zip(counts) {
            if node.kind == Kind::Entity {
                let mut spoken = node.spoken.clone();
                if let Some(caption) = &node.caption {
                    spoken = format!("{spoken}, {caption}");
                }
                node.spoken = format!(
                    "{spoken}, {} in this graph",
                    results::plural(count, "relation")
                );
            }
        }
    }

    pub fn find(&self, key: &str) -> Option<usize> {
        self.index.get(key).copied()
    }

    pub fn relations(&self) -> usize {
        self.rows.len()
    }

    pub fn entities(&self) -> usize {
        self.nodes.iter().filter(|n| n.kind == Kind::Entity).count()
    }

    /// The nodes a selection is about: a relation with its parties, or a party with every
    /// relation it is in and their parties. The rest steps back.
    pub fn context(&self, node: usize) -> BTreeSet<usize> {
        let mut near = BTreeSet::from([node]);
        let relations: Vec<usize> = if self.nodes[node].kind == Kind::Relation {
            vec![node]
        } else {
            self.links
                .iter()
                .filter(|link| link.party == node)
                .map(|link| link.relation)
                .collect()
        };
        for relation in relations {
            near.insert(relation);
            near.extend(
                self.links
                    .iter()
                    .filter(|link| link.relation == relation)
                    .map(|link| link.party),
            );
        }
        near
    }

    /// How many relations of this graph an entity is in.
    pub fn degree(&self, node: usize) -> usize {
        self.links.iter().filter(|link| link.party == node).count()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    fn party(entity: &str) -> Value {
        json!({"kind": "entity", "entity_id": entity})
    }
    pub(crate) fn relation(id: &str, kind: &str, roles: Value, polarity: &str) -> Value {
        json!({
            "relation_id": id,
            "relation": {"relation_type": kind, "roles": roles, "polarity": polarity,
                         "modality": "asserted", "qualifiers": {}},
            "assertions": [{"publication_status": "active", "stale": false}],
        })
    }
    /// Three projects, one supplier contract with three parties and a literal, and a negation.
    pub(crate) fn answer() -> Value {
        json!({
            "entities": [
                {"entity_id": "entity:a", "kind": "project", "labels": ["Project A"]},
                {"entity_id": "entity:b", "kind": "library", "labels": ["Library B"]},
                {"entity_id": "entity:c", "kind": "company", "labels": ["Supplier C"]},
                {"entity_id": "entity:lonely", "kind": "project", "labels": ["Lonely"]},
            ],
            "relations": [
                relation("relation:1", "core:DEPENDS_ON@1",
                    json!({"dependent": [party("entity:a")], "dependency": [party("entity:b")]}),
                    "positive"),
                relation("relation:2", "core:SUPPLIES@1",
                    json!({"supplier": [party("entity:c")], "customer": [party("entity:a")],
                           "product": [party("entity:b")],
                           "since": [{"kind": "literal", "literal": {"type": "string", "value": "2026"}}]}),
                    "positive"),
                relation("relation:3", "core:DEPENDS_ON@1",
                    json!({"dependent": [party("entity:b")], "dependency": [party("entity:c")]}),
                    "negative"),
            ],
            "truncated": false,
            "snapshot_tick": 7,
        })
    }

    #[test]
    fn every_entity_appears_once_and_every_relation_is_a_node_of_its_own() {
        let model = Model::of(&answer()).unwrap();
        assert_eq!((model.entities(), model.relations()), (4, 3));
        let contract = model.find("relation:2").unwrap();
        let parties: Vec<_> = model
            .links
            .iter()
            .filter(|link| link.relation == contract)
            .map(|link| (link.role.as_str(), model.nodes[link.party].label.as_str()))
            .collect();
        assert_eq!(
            parties,
            [
                ("Customer", "Project A"),
                ("Product", "Library B"),
                ("Since", "2026"),
                ("Supplier", "Supplier C")
            ]
        );
        let lonely = model.find("entity:lonely").unwrap();
        assert_eq!(model.degree(lonely), 0, "a matching entity keeps its place");
    }

    #[test]
    fn a_literal_belongs_to_its_relation_and_connects_nothing_else() {
        let mut value = answer();
        let since = json!([{"kind": "literal", "literal": {"type": "string", "value": "2026"}}]);
        value["relations"][0]["relation"]["roles"]["since"] = since;
        let model = Model::of(&value).unwrap();
        let literals: Vec<_> = model
            .nodes
            .iter()
            .filter(|node| node.kind == Kind::Literal)
            .map(|node| node.key.as_str())
            .collect();
        assert_eq!(literals, ["relation:1#2", "relation:2#2"]);
    }

    #[test]
    fn a_negation_is_said_before_the_relation_and_shown_in_its_label() {
        let model = Model::of(&answer()).unwrap();
        let negated = &model.nodes[model.find("relation:3").unwrap()];
        assert_eq!(negated.label, "Not · Depends on");
        assert!(
            negated.spoken.starts_with("Not, Depends on: "),
            "{}",
            negated.spoken
        );
        assert!(negated.spoken.contains("Library B as dependent"));
        let plain = &model.nodes[model.find("relation:1").unwrap()];
        assert_eq!(plain.label, "Depends on");
        let a = &model.nodes[model.find("entity:a").unwrap()];
        assert_eq!(a.spoken, "Project A, Project, 2 relations in this graph");
    }

    #[test]
    fn a_selection_brings_forward_what_it_is_about() {
        let model = Model::of(&answer()).unwrap();
        let at = |key| model.find(key).unwrap();
        let negation = model.context(at("relation:3"));
        assert_eq!(
            negation,
            BTreeSet::from([at("relation:3"), at("entity:b"), at("entity:c")])
        );
        let a = model.context(at("entity:a"));
        assert!(a.contains(&at("relation:2")) && a.contains(&at("entity:c")));
        assert!(
            !a.contains(&at("relation:3")),
            "B's negation is not about A"
        );
    }

    #[test]
    fn past_the_node_bound_the_rest_is_counted_not_drawn() {
        let relations: Vec<Value> = (0..200)
            .map(|i| {
                relation(
                    &format!("relation:{i}"),
                    "core:KNOWS@1",
                    json!({"a": [party(&format!("entity:{i}a"))], "b": [party(&format!("entity:{i}b"))]}),
                    "positive",
                )
            })
            .collect();
        let model = Model::of(&json!({"entities": [], "relations": relations})).unwrap();
        assert!(model.nodes.len() <= MAX_NODES);
        assert_eq!(model.relations() + model.hidden_relations, 200);
        assert!(model.hidden_relations > 0);
    }

    #[test]
    fn an_answer_without_relations_has_no_graph() {
        assert!(Model::of(&json!({"status": "ok"})).is_none());
    }
}
