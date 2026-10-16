//! A typed semantic extraction proposal. No ACL, registry, egress or publishing authority.
use super::admission::ExtractedData;
use crate::answer::llm::{LocalLlm, ModelUse};
use crate::error::{require, Error, Result};
use crate::relation::{query::EntityView, RelationType, RoleValue};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub fn extract(
    model: &dyn LocalLlm,
    source_key: &str,
    text: &str,
    entities: &[EntityView],
    types: &[RelationType],
) -> Result<ExtractedData> {
    require(
        text.len() <= 262_144,
        "LIMIT_EXCEEDED",
        "Semantic source exceeds text budget",
    )?;
    let namespace = format!(
        "source:{}",
        &blake3::hash(source_key.as_bytes()).to_hex().to_string()[..32]
    );
    let visible: Vec<_> = entities.iter().take(128).collect();
    let input = json!({"source_namespace":namespace,"source_text":text,"existing_entities":visible,"registered_types":types,"schema":schemars::schema_for!(ExtractedData),"complete_scope":false});
    let value = model.generate(ModelUse::Extract, &input)?;
    let mut data: ExtractedData = serde_json::from_value(value)
        .map_err(|_| Error::new("MODEL_OUTPUT_INVALID", "Extraction schema is invalid"))?;
    data.validate_bounds()?;
    let mut local = BTreeMap::new();
    for entity in &data.entities {
        require(
            entity.namespace == namespace,
            "MODEL_OUTPUT_INVALID",
            "Model tried to choose an unapproved identity namespace",
        )?;
        require(
            local
                .insert(format!("local:{}", entity.canonical_key), entity.id()?)
                .is_none(),
            "MODEL_OUTPUT_INVALID",
            "Duplicate local entity key",
        )?;
    }
    let known: BTreeSet<_> = visible.iter().map(|e| e.entity_id.as_str()).collect();
    for assertion in &mut data.assertions {
        // Model output cannot attest provenance; a source could otherwise make it cite another
        // capture. Technical origin comes from the capture being published.
        require(
            assertion.evidence.is_empty(),
            "MODEL_OUTPUT_INVALID",
            "Model may not attach evidence",
        )?;
        for value in assertion.relation.roles.values_mut().flatten() {
            if let RoleValue::Entity { entity_id } = value {
                if let Some(id) = local.get(entity_id) {
                    *entity_id = id.clone();
                } else {
                    require(
                        known.contains(entity_id.as_str()),
                        "MODEL_OUTPUT_INVALID",
                        "Model invented an entity identity",
                    )?;
                }
            }
        }
    }
    Ok(data)
}
