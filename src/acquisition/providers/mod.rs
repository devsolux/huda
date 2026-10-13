//! Fixed provider capabilities. Egress, registration and admission are separate permissions.
pub mod wikimedia;
use crate::acquisition::admission::{SourceBatch, SourcePolicy};
use crate::error::{require, Error, Result};
use crate::relation::*;
use crate::storage::{audit, read_epochs, recheck, tick, Store};
use rusqlite::{params, Connection, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderPolicy {
    pub provider_id: String,
    pub scope: String,
    pub allowed_actors: BTreeSet<String>,
    pub languages: BTreeSet<String>,
    /// Explicitly approved public phrases, never inferred from the question or graph ACL.
    pub approved_public_terms: BTreeSet<String>,
    #[serde(default)]
    pub may_fetch: bool,
    #[serde(default)]
    pub auto_register_provider_types: bool,
    #[serde(default)]
    pub auto_admit_structured: bool,
    #[serde(default)]
    pub auto_admit_semantic: bool,
    #[serde(default)]
    pub semantic_types: BTreeSet<String>,
    /// Real operator contact is required before requests, not a fictitious contact address.
    pub user_agent: String,
    pub ttl_seconds: u64,
}
impl ProviderPolicy {
    pub fn validate(&self) -> Result<()> {
        require(
            matches!(self.provider_id.as_str(), "wikidata" | "wikipedia"),
            "INVALID_INPUT",
            "Provider is not implemented",
        )?;
        identifier(&self.scope)?;
        require(
            !self.allowed_actors.is_empty() && self.allowed_actors.len() <= 16,
            "INVALID_INPUT",
            "Provider needs a bounded actor allowlist",
        )?;
        require(
            !self.languages.is_empty()
                && self
                    .languages
                    .iter()
                    .all(|l| matches!(l.as_str(), "en" | "de" | "tr" | "ar")),
            "INVALID_INPUT",
            "Provider language is not qualified",
        )?;
        require(
            self.approved_public_terms.len() <= 64 && self.semantic_types.len() <= 64,
            "LIMIT_EXCEEDED",
            "Provider policy is too large",
        )?;
        for term in &self.approved_public_terms {
            text(term, 256)?;
        }
        for actor in &self.allowed_actors {
            identifier(actor)?;
        }
        for ty in &self.semantic_types {
            text(ty, 256)?;
        }
        text(&self.user_agent, 256)?;
        require(
            self.user_agent.is_ascii()
                && !self.user_agent.chars().any(char::is_control)
                && self
                    .user_agent
                    .contains(&format!("Huda/{}", crate::VERSION))
                && (self.user_agent.contains("https://") || self.user_agent.contains('@')),
            "INVALID_INPUT",
            "User-Agent must identify this Huda version and the operator's contact",
        )?;
        require(
            (60..=604800).contains(&self.ttl_seconds),
            "INVALID_INPUT",
            "TTL must be between one minute and seven days",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderPolicyRequest {
    pub policy: ProviderPolicy,
    pub expected_policy_epoch: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OfflineRequest {
    pub offline: bool,
    pub expected_policy_epoch: u64,
}

/// Only constructed after validating a fixed provider's property response. Not client input.
#[derive(Debug, Clone)]
pub(crate) struct PropertyMetadata {
    pub pid: String,
    pub datatype: String,
    pub revision: u64,
    pub label: String,
    pub sha256: String,
}

pub(crate) fn policy(conn: &Connection, provider: &str) -> Result<ProviderPolicy> {
    let json: Option<String> = conn
        .query_row(
            "SELECT policy_json FROM knowledge_provider_policies WHERE provider_id=?1",
            [provider],
            |r| r.get(0),
        )
        .optional()?;
    let policy: ProviderPolicy = serde_json::from_str(
        &json.ok_or_else(|| Error::new("POLICY_DENIED", "No provider consent is configured"))?,
    )?;
    policy.validate()?;
    Ok(policy)
}
pub(crate) fn check_egress(
    conn: &Connection,
    caller: &Caller,
    provider: &str,
    language: &str,
    term: &str,
    epoch: u64,
) -> Result<ProviderPolicy> {
    recheck(conn, caller, epoch)?;
    caller.require("expand")?;
    require(!read_epochs(conn)?.offline, "OFFLINE", "Profile is offline")?;
    let p = policy(conn, provider)?;
    require(
        p.may_fetch
            && p.allowed_actors.contains(&caller.actor)
            && caller.write(&p.scope)
            && p.languages.contains(language),
        "POLICY_DENIED",
        "Provider read is not authorized",
    )?;
    require(
        p.approved_public_terms.contains(term),
        "POLICY_DENIED",
        "The minimized search phrase has not been approved for disclosure",
    )?;
    Ok(p)
}
impl Store {
    pub fn set_provider_policy(
        &mut self,
        caller: &Caller,
        request: ProviderPolicyRequest,
    ) -> Result<()> {
        caller.require("policy")?;
        request.policy.validate()?;
        require(
            caller.write(&request.policy.scope),
            "PERMISSION_DENIED",
            "Provider scope is not writable",
        )?;
        let tx = self.begin(caller, request.expected_policy_epoch)?;
        for actor in &request.policy.allowed_actors {
            require(
                tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM principals WHERE actor=?1 AND enabled=1)",
                    [actor],
                    |r| r.get(0),
                )?,
                "INVALID_INPUT",
                "Unknown provider actor",
            )?;
        }
        let t = tick(&tx)?;
        tx.execute("INSERT INTO knowledge_provider_policies(provider_id,policy_json,version) VALUES(?1,?2,1) ON CONFLICT(provider_id) DO UPDATE SET policy_json=excluded.policy_json,version=version+1", params![request.policy.provider_id, serde_json::to_string(&request.policy)?])?;
        tx.execute(
            "UPDATE system_meta SET policy_epoch=policy_epoch+1 WHERE singleton=1",
            [],
        )?;
        audit(
            &tx,
            t,
            caller,
            "provider_policy",
            &request.policy.provider_id,
        )?;
        caller.check()?;
        tx.commit()?;
        Ok(())
    }
    pub fn set_offline(&mut self, caller: &Caller, request: OfflineRequest) -> Result<()> {
        caller.require("policy")?;
        let tx = self.begin(caller, request.expected_policy_epoch)?;
        let t = tick(&tx)?;
        tx.execute(
            "UPDATE system_meta SET offline=?1,policy_epoch=policy_epoch+1 WHERE singleton=1",
            [request.offline],
        )?;
        audit(&tx, t, caller, "offline_policy", "profile")?;
        caller.check()?;
        tx.commit()?;
        Ok(())
    }
    pub fn provider_status(&self, caller: &Caller) -> Result<serde_json::Value> {
        caller.require("read")?;
        let mut entries = Vec::new();
        for id in ["wikidata", "wikipedia"] {
            let p = policy(&self.conn, id)
                .ok()
                .filter(|p| p.allowed_actors.contains(&caller.actor) && caller.read(&p.scope));
            let configured = p.is_some();
            let mut value = serde_json::json!({"provider_id":id,"configured":configured,"may_fetch":p.as_ref().is_some_and(|x| x.may_fetch),"capabilities":if id=="wikidata" {vec!["structured_entity","structured_relation"]} else {vec!["encyclopedic_text"]}});
            if let Some(p) = p {
                value["auto_register_provider_types"] = p.auto_register_provider_types.into();
                value["auto_admit_structured"] = p.auto_admit_structured.into();
                value["auto_admit_semantic"] = p.auto_admit_semantic.into();
                if caller.can("policy") {
                    value["policy"] = serde_json::to_value(p)?;
                }
            }
            entries.push(value);
        }
        Ok(serde_json::json!({"offline":self.epochs()?.offline,"providers":entries}))
    }
}

/// Called only inside the sole writer's publication transaction. It cannot grant egress.
pub(crate) fn authorize_capture(
    conn: &Connection,
    caller: &Caller,
    batch: &SourceBatch,
) -> Result<()> {
    recheck(conn, caller, batch.policy_epoch)?;
    require(
        !read_epochs(conn)?.offline,
        "OFFLINE",
        "Offline policy blocks stale publication",
    )?;
    let provider = batch
        .capture
        .provider_id
        .as_deref()
        .ok_or_else(|| Error::invalid("Missing provider"))?;
    let p = policy(conn, provider)?;
    require(
        p.may_fetch && p.allowed_actors.contains(&caller.actor) && caller.write(&p.scope),
        "POLICY_DENIED",
        "Provider publication was revoked",
    )?;
    // An immutable capture may be retained as proposals without auto-admission.
    let mut allowed_types = p.semantic_types.clone();
    for metadata in &batch.provider_properties {
        require(
            provider == "wikidata"
                && wikimedia::external_id(&metadata.pid, b'P')
                && metadata.revision > 0
                && metadata.sha256.len() == 64,
            "INVALID_INPUT",
            "Unqualified provider property metadata",
        )?;
        require(
            matches!(metadata.datatype.as_str(), "wikibase-item" | "string"),
            "TYPE_UNKNOWN",
            "Property datatype needs qualification",
        )?;
        let ty = provider_type(metadata);
        let existing: Option<String> = conn
            .query_row(
                "SELECT definition_json FROM relation_types WHERE name=?1 AND version=1",
                [&ty.name],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing: RelationType = serde_json::from_str(&existing)?;
            require(
                existing.roles == ty.roles,
                "TYPE_REGISTRY_CONFLICT",
                "Provider datatype changed; explicit registry review is required",
            )?;
            allowed_types.insert(ty.key());
        } else if p.auto_register_provider_types {
            crate::relation::registry::validate_type(&ty)?;
            let t = tick(conn)?;
            conn.execute("INSERT INTO relation_types(name,version,definition_json,tx_tick) VALUES(?1,1,?2,?3)", params![ty.name, serde_json::to_string(&ty)?, t])?;
            // Appending an approved restricted type does not change permissions. Policy changes do.
            conn.execute(
                "UPDATE system_meta SET registry_epoch=registry_epoch+1 WHERE singleton=1",
                [],
            )?;
            audit(conn, t, caller, "provider_type_registration", &ty.key())?;
            allowed_types.insert(ty.key());
        }
    }
    // Keep unregistered predicates as proposals at the common validator, not fake core mappings.
    if allowed_types.is_empty() {
        allowed_types.insert("provider:UNREGISTERED@1".into());
    }
    let source = SourcePolicy {
        source_key: batch.source_key.clone(),
        scope: p.scope.clone(),
        allowed_actors: p.allowed_actors.clone(),
        allowed_types,
        entity_namespaces: if provider == "wikidata" {
            BTreeSet::from(["wikidata".into()])
        } else {
            BTreeSet::from([format!(
                "source:{}",
                &blake3::hash(batch.source_key.as_bytes())
                    .to_hex()
                    .to_string()[..32]
            )])
        },
        auto_admit_structured: p.auto_admit_structured,
        auto_admit_semantic: p.auto_admit_semantic,
    };
    let existing: Option<(String, String, bool)> = conn
        .query_row(
            "SELECT owner,scope,withdrawn FROM sources WHERE source_key=?1",
            [&batch.source_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((owner, scope, withdrawn)) = existing {
        require(
            owner == caller.actor && scope == p.scope && !withdrawn,
            "POLICY_DENIED",
            "Source ownership or withdrawal blocks publication",
        )?;
        conn.execute(
            "UPDATE sources SET policy_json=?2 WHERE source_key=?1",
            params![source.source_key, serde_json::to_string(&source)?],
        )?;
    } else {
        conn.execute("INSERT INTO sources(source_key,owner,scope,policy_json,generation,withdrawn) VALUES(?1,?2,?3,?4,0,0)", params![source.source_key, caller.actor, source.scope, serde_json::to_string(&source)?])?;
    }
    Ok(())
}
fn provider_type(metadata: &PropertyMetadata) -> RelationType {
    let subject = RoleSpec {
        entity_kinds: BTreeSet::from(["wikidata_item".into()]),
        literal_types: BTreeSet::new(),
        min: 1,
        max: 1,
        ordered: false,
    };
    let value = if metadata.datatype == "wikibase-item" {
        subject.clone()
    } else {
        RoleSpec {
            entity_kinds: BTreeSet::new(),
            literal_types: BTreeSet::from(["string".into()]),
            min: 1,
            max: 1,
            ordered: false,
        }
    };
    RelationType { name:format!("wikidata:{}", metadata.pid),version:1,description:format!("Wikidata {}: {}. Restricted mapping v1; metadata revision {}; SHA-256 {}. No core mapping or inferred inverse/transitivity.",metadata.pid,metadata.label,metadata.revision,metadata.sha256),roles:BTreeMap::from([("subject".into(),subject),("value".into(),value)]),qualifiers:BTreeMap::new(),traversals:if metadata.datatype=="wikibase-item" {vec![Traversal{from_role:"subject".into(),to_role:"value".into(),bidirectional:false}]}else{Vec::new()},allow_conditions:false,allow_negation:false,allow_valid_time:false }
}
