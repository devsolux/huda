pub mod context;
pub mod llm;

use crate::error::{require, Error, Result};
use crate::relation::{self, condition::Facts, query::*, Literal, Modality};
use context::RelationReference;
use llm::{LocalLlm, ModelUse};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExpansionMode {
    #[default]
    Off,
    Suggest,
    Auto,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AskRequest {
    pub question: String,
    #[serde(default = "english")]
    pub language: String,
    #[serde(default)]
    pub expansion_mode: ExpansionMode,
    #[serde(default)]
    pub valid_at: Option<i64>,
    #[serde(default)]
    pub facts: Facts,
}
fn english() -> String {
    "en".into()
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnswerStatus {
    Answered,
    Partial,
    NoRelationContext,
    ModelUnavailable,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskResponse {
    pub status: AnswerStatus,
    pub answer: Option<String>,
    pub relation_refs: Vec<RelationReference>,
    pub derived_paths: Vec<DerivedPath>,
    pub inferences: Vec<serde_json::Value>,
    pub goal_coverage: GoalCoverageReport,
    pub unknowns: Vec<String>,
    pub conflicts: Vec<Conflict>,
    pub truncated: bool,
    pub degraded_components: Vec<String>,
    pub snapshot_tick: u64,
    pub relation_context_hash: String,
    pub knowledge_needs: Vec<KnowledgeNeed>,
    pub expansion: Option<crate::acquisition::expand::JobView>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelAnswer {
    answer: String,
    used_markers: Vec<String>,
}

/// Semantic interpretation belongs here once. The controller receives its typed result unchanged.
pub fn understand(
    request: &AskRequest,
    entities: &[EntityView],
    types: &[relation::RelationType],
    model: &dyn LocalLlm,
) -> Result<GoalSpec> {
    relation::text(&request.question, 4096)?;
    let mut goal = if model.available() {
        let input = json!({"question":request.question,"language":request.language,"valid_at":request.valid_at,"facts":request.facts,"visible_entities":entities.iter().take(128).collect::<Vec<_>>(),"registered_types":types,"schema":schemars::schema_for!(GoalSpec)});
        match model.generate(ModelUse::Understand, &input) {
            Ok(value) => serde_json::from_value::<GoalSpec>(value).map_err(|_| {
                Error::new(
                    "MODEL_OUTPUT_INVALID",
                    "Query understanding did not produce a valid GoalSpec",
                )
            })?,
            Err(error) if error.code == "MODEL_UNAVAILABLE" => {
                deterministic_goal(request, entities)?
            }
            Err(error) => return Err(error),
        }
    } else {
        deterministic_goal(request, entities)?
    };
    // Policy-relevant fields do not exist in GoalSpec. Hard bounds and real identities are rechecked in Query Core.
    goal.language = request.language.clone();
    goal.valid_at = request.valid_at;
    goal.facts = request.facts.clone();
    goal.goal_id = relation::stable_id(
        "goal",
        &(
            &goal.seeds,
            &goal.relation_type_filter,
            &goal.target_slots,
            &goal.qualifier_filter,
            &goal.language,
            goal.valid_at,
            &goal.facts,
            &goal.modality,
        ),
    )?;
    goal.validate()?;
    Ok(goal)
}
fn deterministic_goal(request: &AskRequest, entities: &[EntityView]) -> Result<GoalSpec> {
    let question = request.question.trim();
    let lower = question.to_lowercase();
    let labels: Vec<String> = if lower.starts_with("explain the relationships between ") {
        split_seeds(&question["explain the relationships between ".len()..])
    } else if lower.starts_with("how are ") && lower.trim_end_matches('?').ends_with(" related") {
        // "How are related?" overlaps prefix and suffix; an empty range names no seed.
        let end = question
            .trim_end_matches('?')
            .len()
            .saturating_sub(" related".len());
        split_seeds(question.get("how are ".len()..end).unwrap_or_default())
    } else {
        let mut matches: Vec<String> = entities
            .iter()
            .flat_map(|e| e.labels.iter())
            .filter(|label| bounded_label_match(&lower, &label.to_lowercase()))
            .cloned()
            .collect();
        matches.sort_by_key(|label| std::cmp::Reverse(label.len()));
        let mut selected: Vec<String> = Vec::new();
        for label in matches {
            if !selected
                .iter()
                .any(|old| old.to_lowercase().contains(&label.to_lowercase()))
            {
                selected.push(label);
            }
        }
        if selected.is_empty() {
            relation::text(question, 256)?;
            vec![question.trim_end_matches('?').to_owned()]
        } else {
            selected
        }
    };
    require(
        (1..=8).contains(&labels.len()),
        "INVALID_INPUT",
        "Use a bounded goal with one to eight seed labels",
    )?;
    let seeds: Vec<Seed> = labels
        .into_iter()
        .enumerate()
        .map(|(i, label)| Seed {
            seed_id: format!("s{}", i + 1),
            label,
            entity_id: None,
        })
        .collect();
    let depends = lower.contains(" depend on") || lower.contains(" depends on");
    let filter = if depends {
        TypeFilter::Only {
            types: BTreeSet::from(["core:DEPENDS_ON@1".into()]),
        }
    } else {
        TypeFilter::Open
    };
    let predicate = if seeds.len() > 1 {
        SuccessPredicate::TypedConnectionCoveringSeeds
    } else if depends {
        // "What does X depend on?" asks for X's dependencies; "What depends on X?" asks for
        // X's dependents, so a seed named only after the verb fills the dependency role.
        let seed = seeds[0].label.to_lowercase();
        let seed_is_dependency = [" depends on ", " depend on "]
            .iter()
            .filter_map(|verb| lower.find(verb).map(|at| (at, at + verb.len())))
            .min()
            .is_some_and(|(at, end)| lower[end..].contains(&seed) && !lower[..at].contains(&seed));
        let (seed_role, target_role) = if seed_is_dependency {
            ("dependency", "dependent")
        } else {
            ("dependent", "dependency")
        };
        SuccessPredicate::FilledRole {
            seed_role: seed_role.into(),
            target_role: target_role.into(),
        }
    } else {
        SuccessPredicate::RelationAboutSeed
    };
    let mut qualifier_filter = BTreeMap::new();
    if lower.contains("in production") {
        qualifier_filter.insert("environment".into(), Literal::String("production".into()));
    }
    Ok(GoalSpec {
        schema_version: 1,
        goal_id: "pending".into(),
        language: request.language.clone(),
        target_slots: vec![GoalSlot {
            slot_id: "connections".into(),
            predicate,
            seed_ids: seeds.iter().map(|s| s.seed_id.clone()).collect(),
            required: true,
            max_hops: 3,
        }],
        seeds,
        relation_type_filter: filter,
        valid_at: request.valid_at,
        facts: request.facts.clone(),
        modality: Modality::Asserted,
        qualifier_filter,
    })
}
fn split_seeds(text: &str) -> Vec<String> {
    text.trim()
        .trim_end_matches(['?', '.'])
        .replace(", and ", ",")
        .replace(" and ", ",")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
fn bounded_label_match(text: &str, label: &str) -> bool {
    text.match_indices(label).any(|(at, _)| {
        text[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric())
            && text[at + label.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric())
    })
}

pub fn formulate(context: &RelationContext, model: &dyn LocalLlm) -> Result<AskResponse> {
    let refs = context::references(context);
    let mut response = AskResponse {
        status: AnswerStatus::NoRelationContext,
        answer: None,
        relation_refs: refs.clone(),
        derived_paths: context.derived_paths.clone(),
        inferences: Vec::new(),
        goal_coverage: context.goal_coverage.clone(),
        unknowns: context.unknowns.clone(),
        conflicts: context.conflicts.clone(),
        truncated: context.truncated,
        degraded_components: Vec::new(),
        snapshot_tick: context.snapshot_tick,
        relation_context_hash: context.context_hash.clone(),
        knowledge_needs: context.knowledge_needs.clone(),
        expansion: None,
    };
    if context.relations.is_empty() {
        return Ok(response);
    }
    if !model.available() {
        response.status = AnswerStatus::ModelUnavailable;
        response
            .degraded_components
            .push("local_model_not_configured".into());
        return Ok(response);
    }
    let output = match model.generate(
        ModelUse::Answer,
        &json!({"relation_context":context,"relation_markers":refs}),
    ) {
        Ok(value) => value,
        Err(error) if error.code == "MODEL_UNAVAILABLE" => {
            response.status = AnswerStatus::ModelUnavailable;
            response
                .degraded_components
                .push("local_model_unavailable".into());
            return Ok(response);
        }
        Err(error) => return Err(error),
    };
    let answer: ModelAnswer = serde_json::from_value(output)
        .map_err(|_| Error::new("MODEL_OUTPUT_INVALID", "Answer schema is invalid"))?;
    validate_answer(&answer, &refs, context)?;
    response.status = if context.goal_coverage.goal_met
        && context.unknowns.is_empty()
        && context.conflicts.is_empty()
        && !context.truncated
    {
        AnswerStatus::Answered
    } else {
        AnswerStatus::Partial
    };
    response.answer = Some(answer.answer);
    response
        .relation_refs
        .retain(|r| answer.used_markers.contains(&r.marker));
    Ok(response)
}
fn validate_answer(
    answer: &ModelAnswer,
    refs: &[RelationReference],
    context: &RelationContext,
) -> Result<()> {
    require(
        !answer.answer.trim().is_empty() && answer.answer.len() <= 32_768,
        "MODEL_OUTPUT_INVALID",
        "Answer text exceeds bounds or is empty",
    )?;
    let allowed: BTreeSet<_> = refs.iter().map(|r| r.marker.as_str()).collect();
    let used: BTreeSet<_> = answer.used_markers.iter().map(String::as_str).collect();
    require(
        !used.is_empty() && used.len() == answer.used_markers.len() && used.is_subset(&allowed),
        "MODEL_OUTPUT_INVALID",
        "Answer references missing or repeated relation markers",
    )?;
    for marker in &used {
        require(
            answer.answer.contains(&format!("[{marker}]")),
            "MODEL_OUTPUT_INVALID",
            "Declared relation marker is absent from answer",
        )?;
    }
    // Scan every marker in every bracket group, not just the model's declared list, so a
    // compound citation such as `[R1, R99]` cannot carry an unvalidated marker.
    for part in answer.answer.split('[').skip(1) {
        if let Some((group, _)) = part.split_once(']') {
            for marker in group.split(|c: char| !c.is_ascii_alphanumeric()) {
                if marker.len() > 1
                    && marker.starts_with('R')
                    && marker[1..].bytes().all(|b| b.is_ascii_digit())
                {
                    require(
                        used.contains(marker),
                        "MODEL_OUTPUT_INVALID",
                        "Answer contains an unvalidated marker",
                    )?;
                }
            }
        }
    }
    for conflict in &context.conflicts {
        for relation in &conflict.relation_ids {
            let marker = refs
                .iter()
                .find(|r| &r.relation_id == relation)
                .ok_or_else(|| {
                    Error::new("MODEL_OUTPUT_INVALID", "Conflict reference is missing")
                })?;
            require(
                used.contains(marker.marker.as_str()),
                "MODEL_OUTPUT_INVALID",
                "Answer omitted one side of a conflict",
            )?;
        }
    }
    Ok(())
}
