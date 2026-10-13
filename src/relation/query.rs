//! ACL-filtered snapshots, typed traversal and deterministic goal coverage have one owner.
use super::condition::{Facts, Truth};
use super::model::*;
use crate::error::{require, Error, Result};
use crate::storage::{read_epochs, Store};
use rusqlite::{params, Connection, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_ASSERTIONS: usize = 4000;
const MAX_ENTITIES: usize = 2000;
const MAX_VISITED: usize = 512;
const MAX_FANOUT: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EntityView {
    pub entity_id: String,
    pub namespace: String,
    pub kind: String,
    pub canonical_key: String,
    pub labels: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Seed {
    pub seed_id: String,
    pub label: String,
    #[serde(default)]
    pub entity_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum TypeFilter {
    Open,
    Only { types: BTreeSet<String> },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SuccessPredicate {
    TypedConnectionCoveringSeeds,
    RelationAboutSeed,
    FilledRole {
        seed_role: String,
        target_role: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoalSlot {
    pub slot_id: String,
    pub predicate: SuccessPredicate,
    pub seed_ids: Vec<String>,
    pub required: bool,
    pub max_hops: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoalSpec {
    pub schema_version: u32,
    pub goal_id: String,
    pub language: String,
    pub seeds: Vec<Seed>,
    pub relation_type_filter: TypeFilter,
    pub target_slots: Vec<GoalSlot>,
    pub valid_at: Option<i64>,
    #[serde(default)]
    pub facts: Facts,
    #[serde(default)]
    pub modality: Modality,
    #[serde(default)]
    pub qualifier_filter: BTreeMap<String, Literal>,
}
impl GoalSpec {
    pub fn validate(&self) -> Result<()> {
        require(
            self.schema_version == 1
                && (1..=8).contains(&self.seeds.len())
                && (1..=8).contains(&self.target_slots.len()),
            "INVALID_INPUT",
            "Goal must contain 1..8 seeds and slots",
        )?;
        identifier(&self.goal_id)?;
        require(
            matches!(self.language.as_str(), "en" | "de" | "tr" | "ar"),
            "INVALID_INPUT",
            "Unsupported goal language",
        )?;
        let mut ids = BTreeSet::new();
        for seed in &self.seeds {
            identifier(&seed.seed_id)?;
            text(&seed.label, 256)?;
            require(
                ids.insert(seed.seed_id.clone()),
                "INVALID_INPUT",
                "Duplicate seed ID",
            )?;
        }
        let mut slots = BTreeSet::new();
        for slot in &self.target_slots {
            identifier(&slot.slot_id)?;
            require(
                slots.insert(&slot.slot_id)
                    && !slot.seed_ids.is_empty()
                    && slot.seed_ids.len() <= 8
                    && (1..=3).contains(&slot.max_hops),
                "INVALID_INPUT",
                "Invalid slot identifiers or bounds",
            )?;
            require(
                slot.seed_ids.iter().all(|id| ids.contains(id))
                    && slot.seed_ids.iter().collect::<BTreeSet<_>>().len() == slot.seed_ids.len(),
                "INVALID_INPUT",
                "Slot references invalid or repeated seeds",
            )?;
            match &slot.predicate {
                SuccessPredicate::TypedConnectionCoveringSeeds => require(
                    slot.seed_ids.len() >= 2,
                    "INVALID_INPUT",
                    "Connections require at least two seeds",
                )?,
                SuccessPredicate::RelationAboutSeed | SuccessPredicate::FilledRole { .. } => {
                    require(
                        slot.seed_ids.len() == 1,
                        "INVALID_INPUT",
                        "This predicate requires one seed",
                    )?
                }
            }
        }
        require(
            self.target_slots.iter().any(|s| s.required),
            "INVALID_INPUT",
            "At least one required slot is necessary",
        )?;
        require(
            self.facts.len() <= 32,
            "LIMIT_EXCEEDED",
            "Too many condition facts",
        )?;
        for (key, fact) in &self.facts {
            identifier(key)?;
            if let super::condition::Fact::Present(value) = fact {
                value.validate()?;
            }
        }
        require(
            self.qualifier_filter.len() <= 16,
            "LIMIT_EXCEEDED",
            "Too many qualifier filters",
        )?;
        for (key, value) in &self.qualifier_filter {
            identifier(key)?;
            value.validate()?;
        }
        if let TypeFilter::Only { types } = &self.relation_type_filter {
            require(
                !types.is_empty() && types.len() <= 32,
                "INVALID_INPUT",
                "Invalid type filter",
            )?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    Covered,
    Partial,
    Unresolved,
    Ambiguous,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CoverageSlot {
    pub slot_id: String,
    pub state: CoverageState,
    pub coverage_basis: Vec<String>,
    pub reason: String,
    pub remaining_need_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoalCoverageReport {
    pub state: CoverageState,
    pub goal_met: bool,
    pub slots: Vec<CoverageSlot>,
    pub remaining_need_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeNeed {
    pub need_id: String,
    pub slot_id: String,
    pub kind: String,
    pub seed_ids: Vec<String>,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PathStep {
    pub relation_id: String,
    pub from_entity: String,
    pub to_entity: String,
    pub from_role: String,
    pub to_role: String,
    /// Navigation against an arrow is disclosed; it never asserts an inverse relation.
    pub reverse_navigation: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DerivedPath {
    pub path_id: String,
    pub kind: String,
    pub steps: Vec<PathStep>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Conflict {
    pub relation_ids: Vec<String>,
    pub state: String,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationContext {
    pub schema_version: u32,
    pub goal: GoalSpec,
    pub entities: Vec<EntityView>,
    pub relations: Vec<RelationView>,
    pub derived_paths: Vec<DerivedPath>,
    pub goal_coverage: GoalCoverageReport,
    pub knowledge_needs: Vec<KnowledgeNeed>,
    pub conflicts: Vec<Conflict>,
    pub unknowns: Vec<String>,
    pub truncated: bool,
    pub snapshot_tick: u64,
    pub context_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExploreRequest {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub relation_type: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub tx_as_of: Option<u64>,
    #[serde(default)]
    pub include_proposed: bool,
}
fn default_limit() -> usize {
    50
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationRequest {
    pub relation_id: String,
    #[serde(default)]
    pub tx_as_of: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExploreResponse {
    pub entities: Vec<EntityView>,
    pub relations: Vec<RelationView>,
    pub truncated: bool,
    pub snapshot_tick: u64,
}
#[derive(Clone)]
struct Snapshot {
    entities: Vec<EntityView>,
    relations: Vec<RelationView>,
    tick: u64,
    truncated: bool,
}

impl Store {
    pub fn visible_entities(&mut self, caller: &Caller) -> Result<Vec<EntityView>> {
        caller.require("read")?;
        caller.check()?;
        Ok(self.snapshot(caller, None, false)?.entities)
    }
    fn snapshot(
        &mut self,
        caller: &Caller,
        as_of: Option<u64>,
        include_proposed: bool,
    ) -> Result<Snapshot> {
        caller.require("read")?;
        caller.check()?;
        let mut reader = self.open_reader()?;
        let tx = reader.transaction()?;
        let current: u64 = tx.query_row(
            "SELECT tx_tick FROM system_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let at = as_of.unwrap_or(current).min(current);
        let scopes = serde_json::to_string(&caller.read_scopes)?;
        let mut stmt=tx.prepare("SELECT r.relation_id,r.relation_json,a.assertion_id,a.scope,v.assertion_version_id,v.version,v.tx_tick,v.status,v.epistemic_kind,v.origin_json,v.evidence_json FROM relation_assertions a JOIN relations r ON r.relation_id=a.relation_id JOIN assertion_versions v ON v.assertion_id=a.assertion_id WHERE a.scope IN (SELECT value FROM json_each(?1)) AND v.version=(SELECT max(v2.version) FROM assertion_versions v2 WHERE v2.assertion_id=a.assertion_id AND v2.tx_tick<=?2) AND (v.status='active' OR (?3 AND v.status='proposed')) ORDER BY r.relation_id,a.assertion_id LIMIT ?4")?;
        let mut rows = stmt.query(params![
            scopes,
            at,
            include_proposed,
            (MAX_ASSERTIONS + 1) as u64
        ])?;
        let mut relations: BTreeMap<String, RelationView> = BTreeMap::new();
        let mut count = 0;
        let mut visible_tick = 0;
        let mut truncated = false;
        let mut heads = SourceHeads::new();
        while let Some(row) = rows.next()? {
            caller.check()?;
            count += 1;
            if count > MAX_ASSERTIONS {
                truncated = true;
                break;
            }
            let relation_id: String = row.get(0)?;
            let body: String = row.get(1)?;
            let relation: RelationInput = serde_json::from_str(&body)?;
            let origin: Origin = serde_json::from_str(&row.get::<_, String>(9)?)?;
            let stale = source_stale(&tx, &mut heads, &origin, at)?;
            let tick: u64 = row.get(6)?;
            visible_tick = visible_tick.max(tick);
            let assertion = AssertionView {
                assertion_id: row.get(2)?,
                scope: row.get(3)?,
                assertion_version_id: row.get(4)?,
                version: row.get(5)?,
                tx_tick: tick,
                publication_status: serde_json::from_value(serde_json::Value::String(row.get(7)?))?,
                epistemic_kind: serde_json::from_value(serde_json::Value::String(row.get(8)?))?,
                origin,
                stale,
                evidence_refs: serde_json::from_str(&row.get::<_, String>(10)?)?,
            };
            relations
                .entry(relation_id.clone())
                .or_insert_with(|| RelationView {
                    relation_id,
                    relation_version_id: String::new(),
                    relation,
                    assertions: Vec::new(),
                })
                .assertions
                .push(assertion);
        }
        drop(rows);
        drop(stmt);
        for relation in relations.values_mut() {
            let ids: Vec<_> = relation
                .assertions
                .iter()
                .map(|a| &a.assertion_version_id)
                .collect();
            relation.relation_version_id = stable_id("rv-visible", &(&relation.relation_id, ids))?;
        }
        let mut stmt=tx.prepare("SELECT e.entity_id,e.namespace,e.kind,e.canonical_key,l.label,l.tx_tick FROM entities e JOIN entity_labels l ON l.entity_id=e.entity_id WHERE l.scope IN (SELECT value FROM json_each(?1)) AND l.tx_tick<=?2 ORDER BY e.entity_id,l.label LIMIT ?3")?;
        let mut rows = stmt.query(params![scopes, at, (MAX_ENTITIES * 17 + 1) as u64])?;
        let mut entities: BTreeMap<String, EntityView> = BTreeMap::new();
        let mut label_rows = 0;
        while let Some(row) = rows.next()? {
            caller.check()?;
            // The query fetches one row past the label cap to disclose dropped labels.
            label_rows += 1;
            if label_rows > MAX_ENTITIES * 17 {
                truncated = true;
                break;
            }
            let id: String = row.get(0)?;
            if !entities.contains_key(&id) && entities.len() >= MAX_ENTITIES {
                truncated = true;
                break;
            }
            visible_tick = visible_tick.max(row.get::<_, u64>(5)?);
            let entry = entities.entry(id.clone()).or_insert(EntityView {
                entity_id: id,
                namespace: row.get(1)?,
                kind: row.get(2)?,
                canonical_key: row.get(3)?,
                labels: Vec::new(),
            });
            let label: String = row.get(4)?;
            if !entry.labels.contains(&label) {
                entry.labels.push(label);
            }
        }
        drop(rows);
        drop(stmt);
        tx.commit()?;
        Ok(Snapshot {
            entities: entities.into_values().collect(),
            relations: relations.into_values().collect(),
            tick: visible_tick,
            truncated,
        })
    }
    pub fn explore(&mut self, caller: &Caller, request: ExploreRequest) -> Result<ExploreResponse> {
        require(
            (1..=100).contains(&request.limit) && request.query.len() <= 512,
            "INVALID_INPUT",
            "Invalid query bounds",
        )?;
        let snapshot = self.snapshot(caller, request.tx_as_of, request.include_proposed)?;
        let matching: BTreeSet<_> = snapshot
            .entities
            .iter()
            .filter(|e| {
                request.query.is_empty()
                    || e.labels
                        .iter()
                        .any(|l| l.to_lowercase().contains(&request.query.to_lowercase()))
            })
            .map(|e| e.entity_id.clone())
            .collect();
        let mut relations: Vec<_> = snapshot
            .relations
            .into_iter()
            .filter(|r| {
                request
                    .relation_type
                    .as_ref()
                    .is_none_or(|t| &r.relation.relation_type == t)
            })
            .filter(|r| {
                request
                    .entity_id
                    .as_ref()
                    .is_none_or(|id| has_entity(&r.relation, id))
            })
            .filter(|r| {
                request.query.is_empty()
                    || matching.iter().any(|id| has_entity(&r.relation, id))
                    || r.relation
                        .relation_type
                        .to_lowercase()
                        .contains(&request.query.to_lowercase())
            })
            .collect();
        let truncated = snapshot.truncated || relations.len() > request.limit;
        relations.truncate(request.limit);
        let mut relevant = matching;
        if !request.query.is_empty() || request.entity_id.is_some() {
            relevant = relations
                .iter()
                .flat_map(|r| entity_ids(&r.relation))
                .collect();
            if let Some(id) = request.entity_id {
                relevant.insert(id);
            }
        }
        let mut entities: Vec<_> = snapshot
            .entities
            .into_iter()
            .filter(|e| relevant.contains(&e.entity_id))
            .collect();
        let truncated = truncated || entities.len() > request.limit;
        entities.truncate(request.limit);
        Ok(ExploreResponse {
            entities,
            relations,
            truncated,
            snapshot_tick: snapshot.tick,
        })
    }
    pub fn get_relation(
        &mut self,
        caller: &Caller,
        request: RelationRequest,
    ) -> Result<RelationView> {
        // Same filtered authority path; knowing an opaque relation ID confers no visibility.
        caller.require("read")?;
        caller.check()?;
        require(
            request.tx_as_of.is_none_or(|tick| tick <= i64::MAX as u64),
            "INVALID_INPUT",
            "Historical tick is out of range",
        )?;
        let mut view = self.history_at(
            caller,
            &request.relation_id,
            request.tx_as_of.unwrap_or(i64::MAX as u64),
        )?;
        view.assertions.retain(|a| {
            matches!(
                a.publication_status,
                PublicationStatus::Active | PublicationStatus::Proposed
            )
        });
        if view.assertions.is_empty() {
            return Err(Error::not_found());
        }
        view.relation_version_id = stable_id(
            "rv-visible",
            &(
                &view.relation_id,
                view.assertions
                    .iter()
                    .map(|a| &a.assertion_version_id)
                    .collect::<Vec<_>>(),
            ),
        )?;
        Ok(view)
    }
    pub fn history(&mut self, caller: &Caller, id: &str) -> Result<Vec<RelationView>> {
        caller.require("read")?;
        caller.check()?;
        let scopes = serde_json::to_string(&caller.read_scopes)?;
        // Only visible assertion events create history entries; private physical versions are not exposed.
        let mut stmt=self.conn.prepare("SELECT DISTINCT v.tx_tick FROM assertion_versions v JOIN relation_assertions a ON a.assertion_id=v.assertion_id WHERE a.relation_id=?1 AND a.scope IN (SELECT value FROM json_each(?2)) ORDER BY v.tx_tick LIMIT 101")?;
        let ticks = stmt
            .query_map(params![id, scopes], |r| r.get::<_, u64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        require(
            ticks.len() <= 100,
            "LIMIT_EXCEEDED",
            "History exceeds 100 visible revisions; narrow historical reads are required",
        )?;
        if ticks.is_empty() {
            return Err(Error::not_found());
        }
        let mut views: Vec<RelationView> = Vec::new();
        for at in ticks {
            let view = self.history_at(caller, id, at)?;
            if views
                .last()
                .is_none_or(|last| last.relation_version_id != view.relation_version_id)
            {
                views.push(view);
            }
        }
        Ok(views)
    }
    fn history_at(&self, caller: &Caller, id: &str, at: u64) -> Result<RelationView> {
        let body:Option<String>=self.conn.query_row("SELECT r.relation_json FROM relations r WHERE r.relation_id=?1 AND EXISTS(SELECT 1 FROM relation_assertions a JOIN assertion_versions v ON v.assertion_id=a.assertion_id WHERE a.relation_id=r.relation_id AND a.scope IN(SELECT value FROM json_each(?2)) AND v.tx_tick<=?3)",params![id,serde_json::to_string(&caller.read_scopes)?,at],|r|r.get(0)).optional()?;
        let body = body.ok_or_else(Error::not_found)?;
        let mut stmt=self.conn.prepare("SELECT a.assertion_id,a.scope,v.assertion_version_id,v.version,v.tx_tick,v.status,v.epistemic_kind,v.origin_json,v.evidence_json FROM relation_assertions a JOIN assertion_versions v ON v.assertion_id=a.assertion_id WHERE a.relation_id=?1 AND a.scope IN(SELECT value FROM json_each(?2)) AND v.version=(SELECT max(v2.version) FROM assertion_versions v2 WHERE v2.assertion_id=a.assertion_id AND v2.tx_tick<=?3) ORDER BY a.assertion_id LIMIT 4001")?;
        let mut rows = stmt.query(params![id, serde_json::to_string(&caller.read_scopes)?, at])?;
        let mut assertions = Vec::new();
        let mut heads = SourceHeads::new();
        while let Some(row) = rows.next()? {
            caller.check()?;
            require(
                assertions.len() < 4000,
                "LIMIT_EXCEEDED",
                "Relation assertion history is too large",
            )?;
            let origin: Origin = serde_json::from_str(&row.get::<_, String>(7)?)?;
            assertions.push(AssertionView {
                assertion_id: row.get(0)?,
                scope: row.get(1)?,
                assertion_version_id: row.get(2)?,
                version: row.get(3)?,
                tx_tick: row.get(4)?,
                publication_status: serde_json::from_value(serde_json::Value::String(row.get(5)?))?,
                epistemic_kind: serde_json::from_value(serde_json::Value::String(row.get(6)?))?,
                stale: source_stale(&self.conn, &mut heads, &origin, at)?,
                origin,
                evidence_refs: serde_json::from_str(&row.get::<_, String>(8)?)?,
            });
        }
        let version = stable_id(
            "rv-visible",
            &(
                id,
                assertions
                    .iter()
                    .map(|a| &a.assertion_version_id)
                    .collect::<Vec<_>>(),
            ),
        )?;
        Ok(RelationView {
            relation_id: id.into(),
            relation_version_id: version,
            relation: serde_json::from_str(&body)?,
            assertions,
        })
    }
    pub fn query_goal(&mut self, caller: &Caller, mut goal: GoalSpec) -> Result<RelationContext> {
        goal.validate()?;
        let snapshot = self.snapshot(caller, None, false)?;
        let types = self.list_types(caller)?;
        if let TypeFilter::Only { types: requested } = &goal.relation_type_filter {
            require(
                requested
                    .iter()
                    .all(|key| types.iter().any(|t| t.key() == *key)),
                "TYPE_UNKNOWN",
                "Goal references an unregistered type version",
            )?;
        }
        let mut ambiguous = BTreeSet::new();
        let mut resolved = BTreeMap::new();
        for seed in &mut goal.seeds {
            let candidates: Vec<_> = snapshot
                .entities
                .iter()
                .filter(|e| {
                    seed.entity_id.as_ref().map_or_else(
                        || {
                            e.labels.iter().any(|l| l.eq_ignore_ascii_case(&seed.label))
                                || e.canonical_key == seed.label
                        },
                        |id| &e.entity_id == id,
                    )
                })
                .collect();
            if candidates.len() == 1 {
                seed.entity_id = Some(candidates[0].entity_id.clone());
                resolved.insert(seed.seed_id.clone(), candidates[0].entity_id.clone());
            } else {
                seed.entity_id = None;
                if candidates.len() > 1 {
                    ambiguous.insert(seed.seed_id.clone());
                }
            }
        }
        let filtered: Vec<_> = snapshot
            .relations
            .into_iter()
            .filter(|r| match &goal.relation_type_filter {
                TypeFilter::Open => true,
                TypeFilter::Only { types } => types.contains(&r.relation.relation_type),
            })
            .filter(|r| {
                goal.qualifier_filter.iter().all(|(key, wanted)| {
                    r.relation
                        .qualifiers
                        .get(key)
                        .is_none_or(|actual| actual == wanted)
                })
            })
            .collect();
        let mut unknowns = Vec::new();
        let eligible: BTreeSet<_> = filtered
            .iter()
            .filter(|r| {
                let reason = ineligible_reason(r, &goal);
                if let Some(reason) = reason {
                    if resolved.values().any(|e| has_entity(&r.relation, e)) {
                        unknowns.push(format!("{}: {}", r.relation_id, reason));
                    }
                    false
                } else {
                    true
                }
            })
            .map(|r| r.relation_id.clone())
            .collect();
        let mut adjacency: BTreeMap<String, Vec<PathStep>> = BTreeMap::new();
        for relation in &filtered {
            if !eligible.contains(&relation.relation_id) {
                continue;
            }
            if let Some(ty) = types
                .iter()
                .find(|t| t.key() == relation.relation.relation_type)
            {
                for traversal in &ty.traversals {
                    let from = role_entities(&relation.relation, &traversal.from_role);
                    let to = role_entities(&relation.relation, &traversal.to_role);
                    for a in &from {
                        for b in &to {
                            adjacency.entry(a.clone()).or_default().push(PathStep {
                                relation_id: relation.relation_id.clone(),
                                from_entity: a.clone(),
                                to_entity: b.clone(),
                                from_role: traversal.from_role.clone(),
                                to_role: traversal.to_role.clone(),
                                reverse_navigation: false,
                            });
                            adjacency.entry(b.clone()).or_default().push(PathStep {
                                relation_id: relation.relation_id.clone(),
                                from_entity: b.clone(),
                                to_entity: a.clone(),
                                from_role: traversal.to_role.clone(),
                                to_role: traversal.from_role.clone(),
                                reverse_navigation: !traversal.bidirectional,
                            });
                        }
                    }
                }
            }
        }
        for edges in adjacency.values_mut() {
            edges.sort_by(|a, b| {
                (&a.relation_id, &a.to_entity).cmp(&(&b.relation_id, &b.to_entity))
            });
        }
        let mut slots = Vec::new();
        let mut needs = Vec::new();
        let mut paths = BTreeMap::new();
        let mut truncated = snapshot.truncated;
        let mut relevant_ids: BTreeSet<String> = filtered
            .iter()
            .filter(|r| resolved.values().any(|id| has_entity(&r.relation, id)))
            .map(|r| r.relation_id.clone())
            .collect();
        for slot in &goal.target_slots {
            let mut basis = BTreeSet::new();
            let has_ambiguity = slot.seed_ids.iter().any(|id| ambiguous.contains(id));
            let all_resolved = slot.seed_ids.iter().all(|id| resolved.contains_key(id));
            let state = if has_ambiguity {
                CoverageState::Ambiguous
            } else if !all_resolved {
                CoverageState::Unresolved
            } else {
                let start = &resolved[&slot.seed_ids[0]];
                match &slot.predicate {
                    SuccessPredicate::TypedConnectionCoveringSeeds => {
                        let mut covered = true;
                        for target in slot.seed_ids.iter().skip(1).map(|id| &resolved[id]) {
                            let (path, cut) =
                                find_path(&adjacency, start, target, slot.max_hops, caller)?;
                            truncated |= cut;
                            if let Some(steps) = path {
                                for step in &steps {
                                    basis.insert(step.relation_id.clone());
                                    relevant_ids.insert(step.relation_id.clone());
                                }
                                let id = stable_id("path", &steps)?;
                                paths.insert(
                                    id.clone(),
                                    DerivedPath {
                                        path_id: id,
                                        kind: "typed_navigation_not_an_inferred_fact".into(),
                                        steps,
                                    },
                                );
                            } else {
                                covered = false;
                            }
                        }
                        if covered && !truncated {
                            CoverageState::Covered
                        } else {
                            CoverageState::Partial
                        }
                    }
                    SuccessPredicate::RelationAboutSeed | SuccessPredicate::FilledRole { .. } => {
                        for relation in &filtered {
                            if !eligible.contains(&relation.relation_id) {
                                continue;
                            }
                            let matched = match &slot.predicate {
                                SuccessPredicate::FilledRole {
                                    seed_role,
                                    target_role,
                                } => {
                                    role_entities(&relation.relation, seed_role).contains(start)
                                        && relation
                                            .relation
                                            .roles
                                            .get(target_role)
                                            .is_some_and(|v| !v.is_empty())
                                }
                                _ => has_entity(&relation.relation, start),
                            };
                            if matched {
                                basis.insert(relation.relation_id.clone());
                            }
                        }
                        if !basis.is_empty() && !truncated {
                            CoverageState::Covered
                        } else {
                            CoverageState::Partial
                        }
                    }
                }
            };
            let reason=match state { CoverageState::Covered=>"The bounded typed slot predicate is satisfied; this is not a truth/completeness claim",CoverageState::Ambiguous=>"A seed has multiple visible identities",CoverageState::Unresolved=>"A seed has no visible qualified identity",CoverageState::Partial=>"Eligible relations, conditions, valid time or bounded paths do not satisfy the slot" }.to_string();
            let need_ids = if state != CoverageState::Covered {
                let id = stable_id("need", &(&goal.goal_id, &slot.slot_id, &state))?;
                needs.push(KnowledgeNeed {
                    need_id: id.clone(),
                    slot_id: slot.slot_id.clone(),
                    kind: match state {
                        CoverageState::Ambiguous => "ambiguous_identity",
                        CoverageState::Unresolved => "missing_entity_match",
                        _ => "insufficient_goal_coverage",
                    }
                    .into(),
                    seed_ids: slot.seed_ids.clone(),
                    reason: reason.clone(),
                });
                vec![id]
            } else {
                Vec::new()
            };
            slots.push(CoverageSlot {
                slot_id: slot.slot_id.clone(),
                state,
                coverage_basis: basis.into_iter().collect(),
                reason,
                remaining_need_ids: need_ids,
            });
        }
        let relations: Vec<_> = filtered
            .into_iter()
            .filter(|r| relevant_ids.contains(&r.relation_id))
            .collect();
        let conflicts = detect_conflicts(&relations, &goal);
        let goal_met = !truncated
            && conflicts.is_empty()
            && goal.target_slots.iter().filter(|s| s.required).all(|s| {
                slots
                    .iter()
                    .any(|c| c.slot_id == s.slot_id && c.state == CoverageState::Covered)
            });
        let state = if goal_met {
            CoverageState::Covered
        } else if slots.iter().any(|s| s.state == CoverageState::Ambiguous) {
            CoverageState::Ambiguous
        } else if resolved.is_empty() {
            CoverageState::Unresolved
        } else {
            CoverageState::Partial
        };
        let entity_ids: BTreeSet<_> = relations
            .iter()
            .flat_map(|r| entity_ids(&r.relation))
            .chain(resolved.values().cloned())
            .collect();
        let entities = snapshot
            .entities
            .into_iter()
            .filter(|e| entity_ids.contains(&e.entity_id))
            .collect();
        unknowns.sort();
        unknowns.dedup();
        let coverage = GoalCoverageReport {
            state,
            goal_met,
            remaining_need_ids: needs.iter().map(|n| n.need_id.clone()).collect(),
            slots,
        };
        let mut context = RelationContext {
            schema_version: 1,
            goal,
            entities,
            relations,
            derived_paths: paths.into_values().collect(),
            goal_coverage: coverage,
            knowledge_needs: needs,
            conflicts,
            unknowns,
            truncated,
            snapshot_tick: snapshot.tick,
            context_hash: String::new(),
        };
        context.context_hash = crate::answer::context::context_hash(&context)?;
        require(
            canonical_bytes(&context)?.len() <= 524_288,
            "LIMIT_EXCEEDED",
            "Relation context exceeds byte budget; narrow the query",
        )?;
        Ok(context)
    }
    pub fn check_policy(&self, caller: &Caller, epoch: u64) -> Result<()> {
        crate::storage::recheck(&self.conn, caller, epoch)
    }
    pub fn policy_epoch(&self) -> Result<u64> {
        Ok(read_epochs(&self.conn)?.policy)
    }
}
/// Latest capture revision and expiry per source at one tick, read once per query.
type SourceHeads = BTreeMap<String, Option<(String, Option<u64>)>>;
fn source_stale(
    conn: &Connection,
    heads: &mut SourceHeads,
    origin: &Origin,
    at: u64,
) -> Result<bool> {
    let Some(source) = &origin.source_key else {
        return Ok(false);
    };
    let head = if let Some(head) = heads.get(source) {
        head.clone()
    } else {
        // Captures can hold megabytes of payload; read only the revision column and expiry.
        let head = conn.query_row("SELECT revision,json_extract(capture_json,'$.expires_at') FROM source_revisions WHERE source_key=?1 AND tx_tick<=?2 ORDER BY generation DESC LIMIT 1",params![source,at],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<u64>>(1)?))).optional()?;
        heads.insert(source.clone(), head.clone());
        head
    };
    let Some((revision, expires_at)) = head else {
        return Ok(true);
    };
    Ok(origin.source_revision.as_ref() != Some(&revision)
        || expires_at.is_some_and(|until| until <= now_ms()))
}
fn role_entities(input: &RelationInput, role: &str) -> Vec<String> {
    input
        .roles
        .get(role)
        .into_iter()
        .flatten()
        .filter_map(|v| match v {
            RoleValue::Entity { entity_id } => Some(entity_id.clone()),
            _ => None,
        })
        .collect()
}
fn entity_ids(input: &RelationInput) -> Vec<String> {
    input
        .roles
        .values()
        .flatten()
        .filter_map(|v| match v {
            RoleValue::Entity { entity_id } => Some(entity_id.clone()),
            _ => None,
        })
        .collect()
}
fn has_entity(input: &RelationInput, id: &str) -> bool {
    input
        .roles
        .values()
        .flatten()
        .any(|v| matches!(v,RoleValue::Entity{entity_id} if entity_id==id))
}
fn ineligible_reason(view: &RelationView, goal: &GoalSpec) -> Option<&'static str> {
    if !goal
        .qualifier_filter
        .iter()
        .all(|(key, wanted)| view.relation.qualifiers.get(key) == Some(wanted))
    {
        return Some("required qualifier is undetermined");
    }
    if view.relation.polarity != Polarity::Positive {
        return Some("explicit negative assertion; not a positive traversal");
    }
    if view.relation.modality != goal.modality {
        return Some("modality does not match the requested goal");
    }
    match view.relation.condition.evaluate(&goal.facts) {
        Truth::False => return Some("condition is FALSE"),
        Truth::Unknown => return Some("condition is UNKNOWN"),
        Truth::True => {}
    }
    if let Some(at) = goal.valid_at {
        match &view.relation.valid_time {
            Some(ValidTime {
                start: Some(start),
                end: Some(end),
            }) if *start <= at && at < *end => {}
            _ => return Some("domain time is undetermined or outside the requested interval"),
        }
    }
    if !view.assertions.iter().any(|a| {
        a.publication_status == PublicationStatus::Active
            && !a.stale
            && a.epistemic_kind != EpistemicKind::Hypothetical
    }) {
        return Some("no fresh, active, non-hypothetical assertion");
    }
    None
}
fn find_path(
    adjacency: &BTreeMap<String, Vec<PathStep>>,
    start: &str,
    target: &str,
    max_hops: u8,
    caller: &Caller,
) -> Result<(Option<Vec<PathStep>>, bool)> {
    if start == target {
        return Ok((Some(Vec::new()), false));
    }
    let mut queue = VecDeque::from([(start.to_owned(), Vec::<PathStep>::new())]);
    let mut visited = BTreeSet::from([start.to_owned()]);
    let mut cut = false;
    while let Some((node, path)) = queue.pop_front() {
        caller.check()?;
        if path.len() >= usize::from(max_hops) {
            continue;
        }
        if let Some(edges) = adjacency.get(&node) {
            cut |= edges.len() > MAX_FANOUT;
            for edge in edges.iter().take(MAX_FANOUT) {
                let mut next = path.clone();
                next.push(edge.clone());
                if edge.to_entity == target {
                    return Ok((Some(next), cut));
                }
                if visited.len() >= MAX_VISITED {
                    return Ok((None, true));
                }
                if visited.insert(edge.to_entity.clone()) {
                    queue.push_back((edge.to_entity.clone(), next));
                }
            }
        }
    }
    Ok((None, cut))
}
fn detect_conflicts(relations: &[RelationView], goal: &GoalSpec) -> Vec<Conflict> {
    let mut conflicts = Vec::new();
    for (i, a) in relations.iter().enumerate() {
        for b in relations.iter().skip(i + 1) {
            if a.relation.polarity == b.relation.polarity
                || a.relation.relation_type != b.relation.relation_type
                || a.relation.roles != b.relation.roles
                || a.relation.qualifiers != b.relation.qualifiers
                || a.relation.modality != b.relation.modality
            {
                continue;
            }
            if a.relation.condition.evaluate(&goal.facts) == Truth::False
                || b.relation.condition.evaluate(&goal.facts) == Truth::False
            {
                continue;
            }
            let time_known = match (&a.relation.valid_time, &b.relation.valid_time) {
                (
                    Some(ValidTime {
                        start: Some(a0),
                        end: Some(a1),
                    }),
                    Some(ValidTime {
                        start: Some(b0),
                        end: Some(b1),
                    }),
                ) => {
                    if a1 <= b0 || b1 <= a0 {
                        continue;
                    }
                    true
                }
                _ => false,
            };
            let definite = time_known
                && a.relation.condition.evaluate(&goal.facts) == Truth::True
                && b.relation.condition.evaluate(&goal.facts) == Truth::True;
            conflicts.push(Conflict {
                relation_ids: vec![a.relation_id.clone(), b.relation_id.clone()],
                state: if definite {
                    "conflict"
                } else {
                    "potential_conflict"
                }
                .into(),
                reason: if definite {
                    "Opposite explicit polarities with compatible roles and overlapping validity"
                } else {
                    "Opposite polarities; time or condition compatibility is undetermined"
                }
                .into(),
            });
        }
    }
    conflicts
}
