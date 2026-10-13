use crate::error::{require, Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Transport-created identity. Deliberately not Deserialize: request JSON cannot create rights.
#[derive(Debug, Clone)]
pub struct Caller {
    pub actor: String,
    pub read_scopes: BTreeSet<String>,
    pub write_scopes: BTreeSet<String>,
    pub permissions: BTreeSet<String>,
    pub deadline_ms: u64,
    pub cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Caller {
    pub fn check(&self) -> Result<()> {
        require(
            !self.cancelled.load(std::sync::atomic::Ordering::Acquire),
            "CANCELLED",
            "Operation was cancelled",
        )?;
        require(
            now_ms() <= self.deadline_ms,
            "DEADLINE_EXCEEDED",
            "Operation deadline exceeded",
        )
    }
    pub fn can(&self, permission: &str) -> bool {
        self.permissions.contains(permission)
    }
    pub fn read(&self, scope: &str) -> bool {
        self.read_scopes.contains(scope)
    }
    pub fn write(&self, scope: &str) -> bool {
        self.can("write") && self.write_scopes.contains(scope)
    }
    pub fn require(&self, permission: &str) -> Result<()> {
        if self.can(permission) {
            Ok(())
        } else {
            Err(Error::denied())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EntityInput {
    pub namespace: String,
    pub kind: String,
    pub canonical_key: String,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}
impl EntityInput {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.namespace)?;
        identifier(&self.kind)?;
        text(&self.canonical_key, 1024)?;
        text(&self.name, 512)?;
        require(
            self.aliases.len() <= 16,
            "LIMIT_EXCEEDED",
            "Too many aliases",
        )?;
        for alias in &self.aliases {
            text(alias, 512)?;
        }
        Ok(())
    }
    pub fn id(&self) -> Result<String> {
        self.validate()?;
        stable_id("ent", &(&self.namespace, &self.kind, &self.canonical_key))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Literal {
    String(String),
    Integer(i64),
    Boolean(bool),
    /// Decimal strings avoid platform-dependent floating-point canonicalization.
    Decimal(String),
    Timestamp(i64),
    Quantity {
        amount: String,
        unit: String,
    },
}
impl Literal {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::String(_) => "string",
            Self::Integer(_) => "integer",
            Self::Boolean(_) => "boolean",
            Self::Decimal(_) => "decimal",
            Self::Timestamp(_) => "timestamp",
            Self::Quantity { .. } => "quantity",
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::String(v) => text(v, 4096),
            Self::Decimal(v) => validate_decimal(v),
            Self::Quantity { amount, unit } => {
                validate_decimal(amount)?;
                text(unit, 256)
            }
            _ => Ok(()),
        }
    }
}
fn validate_decimal(s: &str) -> Result<()> {
    // One canonical spelling: no exponent, leading plus/zeros, negative zero or trailing fractional zeros.
    let unsigned = s.strip_prefix('-').unwrap_or(s);
    let parts: Vec<_> = unsigned.split('.').collect();
    let valid = s.len() <= 96
        && !unsigned.is_empty()
        && parts.len() <= 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && (parts[0] == "0" || !parts[0].starts_with('0'))
        && (parts.len() == 1 || !parts[1].ends_with('0'))
        && s != "-0";
    require(
        valid,
        "INVALID_INPUT",
        "Decimal must have a canonical non-exponent spelling",
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RoleValue {
    Entity { entity_id: String },
    Literal { literal: Literal },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoleSpec {
    #[serde(default)]
    pub entity_kinds: BTreeSet<String>,
    #[serde(default)]
    pub literal_types: BTreeSet<String>,
    pub min: u8,
    pub max: u8,
    #[serde(default)]
    pub ordered: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Traversal {
    pub from_role: String,
    pub to_role: String,
    #[serde(default)]
    pub bidirectional: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RelationType {
    pub name: String,
    pub version: u32,
    pub description: String,
    pub roles: BTreeMap<String, RoleSpec>,
    /// All allowed qualifiers are identity-significant in canonicalizer v1.
    #[serde(default)]
    pub qualifiers: BTreeMap<String, String>,
    #[serde(default)]
    pub traversals: Vec<Traversal>,
    #[serde(default)]
    pub allow_conditions: bool,
    #[serde(default)]
    pub allow_negation: bool,
    #[serde(default)]
    pub allow_valid_time: bool,
}
impl RelationType {
    pub fn key(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    #[default]
    Positive,
    Negative,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    #[default]
    Asserted,
    Planned,
    Possible,
    Counterfactual,
    Quoted,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EpistemicKind {
    #[default]
    Asserted,
    Extracted,
    Hypothetical,
    Inferred,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PublicationStatus {
    Active,
    Proposed,
    Withdrawn,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValidTime {
    /// Half-open UTC interval in milliseconds. None is an unknown bound, not "now".
    pub start: Option<i64>,
    pub end: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RelationInput {
    pub relation_type: String,
    pub roles: BTreeMap<String, Vec<RoleValue>>,
    #[serde(default)]
    pub qualifiers: BTreeMap<String, Literal>,
    #[serde(default)]
    pub polarity: Polarity,
    #[serde(default)]
    pub modality: Modality,
    #[serde(default)]
    pub condition: super::condition::Condition,
    #[serde(default)]
    pub valid_time: Option<ValidTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source_revision: String,
    pub locator: String,
    pub sha256: String,
    #[serde(default)]
    pub approximate: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssertRequest {
    pub relation: RelationInput,
    pub scope: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub epistemic_kind: EpistemicKind,
    pub expected_policy_epoch: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EntityRequest {
    pub entity: EntityInput,
    pub scope: String,
    pub expected_policy_epoch: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WithdrawRequest {
    pub assertion_id: String,
    pub expected_version: u64,
    pub expected_policy_epoch: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegisterTypeRequest {
    pub relation_type: RelationType,
    pub expected_registry_epoch: u64,
    pub expected_policy_epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub kind: String,
    pub actor: String,
    pub operation_id: String,
    pub source_key: Option<String>,
    pub source_revision: Option<String>,
    pub source_generation: Option<u64>,
    pub extraction_scope_key: Option<String>,
    pub extractor_digest: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssertionView {
    pub assertion_id: String,
    pub assertion_version_id: String,
    pub version: u64,
    pub publication_status: PublicationStatus,
    pub epistemic_kind: EpistemicKind,
    pub origin: Origin,
    pub scope: String,
    pub tx_tick: u64,
    pub stale: bool,
    pub evidence_refs: Vec<Evidence>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RelationView {
    pub relation_id: String,
    /// Caller-visible version fingerprint, not the global private assertion-set version.
    pub relation_version_id: String,
    pub relation: RelationInput,
    pub assertions: Vec<AssertionView>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Published {
    pub relation_id: String,
    pub assertion_id: String,
    pub assertion_version: u64,
    pub publication_status: PublicationStatus,
    pub changed: bool,
}

pub fn text(value: &str, max: usize) -> Result<()> {
    require(
        !value.trim().is_empty() && value.len() <= max && !value.chars().any(|c| c == '\0'),
        "INVALID_INPUT",
        "Text is empty, too long or contains NUL",
    )
}
pub fn identifier(value: &str) -> Result<()> {
    require(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_:-./".contains(&c)),
        "INVALID_INPUT",
        "Invalid identifier",
    )
}
pub fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    // serde_json::Map is BTreeMap without preserve_order; nested objects sort recursively.
    Ok(serde_json::to_vec(&serde_json::to_value(value)?)?)
}
pub fn stable_id<T: Serialize>(prefix: &str, value: &T) -> Result<String> {
    Ok(format!(
        "{}:{}",
        prefix,
        blake3::hash(&canonical_bytes(value)?).to_hex()
    ))
}
/// Lowercase hex SHA-256; digest 0.11 output arrays no longer implement LowerHex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub fn random_id(prefix: &str) -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| Error::new("ENTROPY_UNAVAILABLE", "Secure random source unavailable"))?;
    Ok(format!("{prefix}:{}", blake3::hash(&bytes).to_hex()))
}
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}
