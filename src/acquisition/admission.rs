//! Admission DTOs are separate from model output. Source policies are privileged user actions.
use crate::error::{require, Error, Result};
use crate::relation::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourcePolicy {
    pub source_key: String,
    pub scope: String,
    pub allowed_actors: BTreeSet<String>,
    pub allowed_types: BTreeSet<String>,
    pub entity_namespaces: BTreeSet<String>,
    #[serde(default)]
    pub auto_admit_structured: bool,
    #[serde(default)]
    pub auto_admit_semantic: bool,
}
impl SourcePolicy {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.source_key)?;
        identifier(&self.scope)?;
        require(
            !self.allowed_actors.is_empty()
                && self.allowed_actors.len() <= 16
                && !self.allowed_types.is_empty()
                && self.allowed_types.len() <= 128,
            "INVALID_INPUT",
            "Source needs bounded actors and type allowlists",
        )?;
        for actor in &self.allowed_actors {
            identifier(actor)?;
        }
        require(
            !self.entity_namespaces.is_empty() && self.entity_namespaces.len() <= 32,
            "INVALID_INPUT",
            "Source requires an explicit entity namespace allowlist",
        )?;
        for namespace in &self.entity_namespaces {
            identifier(namespace)?;
        }
        for ty in &self.allowed_types {
            text(ty, 256)?;
        }
        Ok(())
    }
    pub fn allows_type(&self, key: &str) -> bool {
        self.allowed_types.contains(key)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdmitSourceRequest {
    pub policy: SourcePolicy,
    pub expected_policy_epoch: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StructuredAssertion {
    pub anchor: String,
    pub relation: RelationInput,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub epistemic_kind: EpistemicKind,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct ExtractedData {
    #[serde(default)]
    pub entities: Vec<EntityInput>,
    #[serde(default)]
    pub assertions: Vec<StructuredAssertion>,
}
impl ExtractedData {
    pub fn validate_bounds(&self) -> Result<()> {
        require(
            self.entities.len() <= 1000 && self.assertions.len() <= 1000,
            "LIMIT_EXCEEDED",
            "Extraction batch exceeds limits",
        )?;
        let mut anchors = BTreeSet::new();
        for entity in &self.entities {
            entity.validate()?;
        }
        for assertion in &self.assertions {
            text(&assertion.anchor, 512)?;
            require(
                anchors.insert((&assertion.anchor, &assertion.relation.relation_type)),
                "INVALID_INPUT",
                "Duplicate assertion anchors in batch",
            )?;
            require(
                assertion.epistemic_kind != EpistemicKind::Inferred,
                "INFERENCE_DISABLED",
                "Persistent inference requires the dependency/invalidation gate",
            )?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourcePayload {
    Relations {
        data: ExtractedData,
    },
    Text {
        text: String,
    },
    Markdown {
        text: String,
    },
    Code {
        text: String,
        language: String,
        relative_path: String,
        root_namespace: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestRequest {
    pub source_key: String,
    pub revision: String,
    pub expected_generation: u64,
    pub extraction_scope_key: String,
    #[serde(default)]
    pub coverage_complete_for_scope: bool,
    pub idempotency_key: String,
    pub expected_policy_epoch: u64,
    pub payload: SourcePayload,
}
impl IngestRequest {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.source_key)?;
        text(&self.revision, 256)?;
        identifier(&self.extraction_scope_key)?;
        text(&self.idempotency_key, 128)?;
        if !matches!(self.payload, SourcePayload::Relations { .. })
            && self.coverage_complete_for_scope
        {
            return Err(Error::invalid("Semantic/code extraction is partial-additive; it cannot attest complete scope coverage"));
        }
        require(
            canonical_bytes(&self.payload)?.len() <= crate::MAX_SOURCE_BYTES,
            "LIMIT_EXCEEDED",
            "Source payload is too large",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WithdrawSourceRequest {
    pub source_key: String,
    pub expected_generation: u64,
    pub expected_policy_epoch: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub source_uri: Option<String>,
    pub provider_id: Option<String>,
    pub external_id: Option<String>,
    pub revision: String,
    pub retrieved_at: u64,
    pub expires_at: Option<u64>,
    pub sha256: String,
    pub license: Option<String>,
    pub attribution: Option<String>,
    /// Immutable normalized bytes, encoded as JSON; never executed or used as policy.
    pub payload: serde_json::Value,
}
impl Capture {
    pub fn local(revision: String, payload: serde_json::Value) -> Result<Self> {
        let bytes = canonical_bytes(&payload)?;
        require(
            bytes.len() <= crate::MAX_SOURCE_BYTES,
            "LIMIT_EXCEEDED",
            "Capture is too large",
        )?;
        Ok(Self {
            source_uri: None,
            provider_id: None,
            external_id: None,
            revision,
            retrieved_at: now_ms(),
            expires_at: None,
            sha256: sha256_hex(&bytes),
            license: None,
            attribution: None,
            payload,
        })
    }
}
/// Internal publication envelope. Never deserialize this from a client/provider/model.
pub(crate) struct SourceBatch {
    pub source_key: String,
    pub expected_generation: u64,
    pub scope_key: String,
    pub complete: bool,
    pub extractor_digest: String,
    pub origin_kind: String,
    pub semantic: bool,
    pub data: ExtractedData,
    pub capture: Capture,
    pub request_key: String,
    pub policy_epoch: u64,
    pub job_id: Option<String>,
    pub provider_properties: Vec<super::providers::PropertyMetadata>,
    pub raw_proposals: Vec<(String, serde_json::Value)>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportResult {
    pub source_key: String,
    pub generation: u64,
    pub published: Vec<Published>,
    pub proposal_ids: Vec<String>,
    pub withdrawn_assertion_ids: Vec<String>,
}
