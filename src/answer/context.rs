//! Deterministic, caller-visible context and reference compilation.
use crate::error::Result;
use crate::relation::{canonical_bytes, query::RelationContext, sha256_hex};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationReference {
    pub marker: String,
    pub relation_id: String,
    pub relation_version_id: String,
    pub assertion_ids: Vec<String>,
}
pub fn references(context: &RelationContext) -> Vec<RelationReference> {
    context
        .relations
        .iter()
        .enumerate()
        .map(|(i, r)| RelationReference {
            marker: format!("R{}", i + 1),
            relation_id: r.relation_id.clone(),
            relation_version_id: r.relation_version_id.clone(),
            assertion_ids: r
                .assertions
                .iter()
                .map(|a| a.assertion_id.clone())
                .collect(),
        })
        .collect()
}
pub fn context_hash(context: &RelationContext) -> Result<String> {
    let mut body = serde_json::to_value(context)?;
    // Global publication ticks and invisible assertion changes must not perturb this fingerprint.
    body.as_object_mut()
        .expect("RelationContext serializes as an object")
        .remove("snapshot_tick");
    body.as_object_mut()
        .expect("RelationContext serializes as an object")
        .remove("context_hash");
    Ok(format!("sha256:{}", sha256_hex(&canonical_bytes(&body)?)))
}
