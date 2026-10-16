//! The semantic publication boundary. All callers, importers and provider jobs converge here.
use super::model::*;
use super::registry::{load_type, normalize_relation};
use crate::acquisition::admission::*;
use crate::error::{require, Error, Result};
use crate::storage::{audit, recheck, tick, Store};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeSet;

impl Store {
    pub fn upsert_entity(&mut self, caller: &Caller, request: EntityRequest) -> Result<String> {
        require(
            caller.write(&request.scope),
            "PERMISSION_DENIED",
            "Entity scope is not writable",
        )?;
        request.entity.validate()?;
        let tx = self.begin(caller, request.expected_policy_epoch)?;
        let t = tick(&tx)?;
        let id = insert_entity(&tx, caller, &request.scope, &request.entity, t)?;
        audit(&tx, t, caller, "upsert_entity", &id)?;
        caller.check()?;
        tx.commit()?;
        Ok(id)
    }
    pub fn assert_relation(
        &mut self,
        caller: &Caller,
        request: AssertRequest,
    ) -> Result<Published> {
        require(
            caller.write(&request.scope),
            "PERMISSION_DENIED",
            "Assertion scope is not writable",
        )?;
        require(
            request.epistemic_kind != EpistemicKind::Inferred,
            "INFERENCE_DISABLED",
            "Persistent inferred assertions are disabled",
        )?;
        text(&request.idempotency_key, 128)?;
        let key = format!("assert:{}", request.idempotency_key);
        // The policy epoch pins when a write may happen, not what was asked: a committed request
        // is answered from its record even after the epoch moved on. Earlier builds hashed it.
        let hash = stable_id(
            "request",
            &(
                &request.relation,
                &request.scope,
                &request.idempotency_key,
                &request.evidence,
                &request.epistemic_kind,
            ),
        )?;
        let legacy_hash = stable_id("request", &request)?;
        let tx = self.begin_unpinned(caller)?;
        if let Some(previous) = replay::<Published>(&tx, caller, &key, &[&hash, &legacy_hash])? {
            tx.commit()?;
            return Ok(previous);
        }
        recheck(&tx, caller, request.expected_policy_epoch)?;
        let (relation, relation_id) =
            normalize_relation(&tx, caller, &request.relation, &request.scope)?;
        validate_evidence(&tx, caller, &request.scope, &request.evidence)?;
        let assertion_id = stable_id("assert", &(&caller.actor, &request.idempotency_key))?;
        let t = tick(&tx)?;
        insert_relation(&tx, &relation_id, &relation, t)?;
        let origin = Origin {
            kind: "user".into(),
            actor: caller.actor.clone(),
            operation_id: String::new(),
            source_key: None,
            source_revision: None,
            source_generation: None,
            extraction_scope_key: None,
            extractor_digest: None,
        };
        let (version, changed) = append_assertion(
            &tx,
            caller,
            AssertionWrite {
                assertion_id: &assertion_id,
                relation_id: &relation_id,
                scope: &request.scope,
                anchor: &request.idempotency_key,
                status: PublicationStatus::Active,
                epistemic: request.epistemic_kind,
                origin,
                evidence: request.evidence,
                tick: t,
            },
        )?;
        snapshot_relation(&tx, &relation_id, t)?;
        let result = Published {
            relation_id,
            assertion_id,
            assertion_version: version,
            publication_status: PublicationStatus::Active,
            changed,
        };
        remember(&tx, caller, &key, &hash, &result)?;
        caller.check()?;
        tx.commit()?;
        Ok(result)
    }
    pub fn withdraw_assertion(
        &mut self,
        caller: &Caller,
        request: WithdrawRequest,
    ) -> Result<Published> {
        caller.require("write")?;
        let tx = self.begin(caller, request.expected_policy_epoch)?;
        let row:Option<(String,String,String,u64,String)>=tx.query_row("SELECT relation_id,actor,scope,current_version,coalesce(source_key,'') FROM relation_assertions WHERE assertion_id=?1 AND actor=?2 AND scope IN(SELECT value FROM json_each(?3))",params![request.assertion_id,caller.actor,serde_json::to_string(&caller.read_scopes)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let (relation_id, actor, scope, version, source) = row.ok_or_else(Error::not_found)?;
        require(caller.write(&scope) && actor==caller.actor && source.is_empty(),"PERMISSION_DENIED","Only the actor can withdraw this direct assertion; imported assertions use source lifecycle")?;
        require(
            version == request.expected_version,
            "VERSION_CONFLICT",
            "Assertion version changed",
        )?;
        let t = tick(&tx)?;
        let new_version = withdraw(&tx, caller, &request.assertion_id, t)?;
        snapshot_relation(&tx, &relation_id, t)?;
        caller.check()?;
        tx.commit()?;
        Ok(Published {
            relation_id,
            assertion_id: request.assertion_id,
            assertion_version: new_version,
            publication_status: PublicationStatus::Withdrawn,
            changed: new_version != version,
        })
    }
    pub fn admit_source(
        &mut self,
        caller: &Caller,
        request: AdmitSourceRequest,
    ) -> Result<SourcePolicy> {
        caller.require("policy")?;
        request.policy.validate()?;
        require(
            caller.write(&request.policy.scope),
            "PERMISSION_DENIED",
            "Source scope is not writable",
        )?;
        let tx = self.begin(caller, request.expected_policy_epoch)?;
        let existing: Option<(String, String, bool)> = tx
            .query_row(
                "SELECT owner,scope,withdrawn FROM sources WHERE source_key=?1",
                [&request.policy.source_key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((owner, scope, _)) = &existing {
            require(
                owner == &caller.actor && scope == &request.policy.scope,
                "PERMISSION_DENIED",
                "A source cannot change owner or scope",
            )
            .map_err(|_| Error::not_found())?;
        }
        for actor in &request.policy.allowed_actors {
            require(
                tx.prepare("SELECT 1 FROM principals WHERE actor=?1 AND enabled=1")?
                    .exists([actor])?,
                "INVALID_INPUT",
                "Source names an unknown or revoked actor",
            )?;
        }
        let t = tick(&tx)?;
        tx.execute("INSERT INTO sources(source_key,owner,scope,policy_json,generation,current_revision) VALUES(?1,?2,?3,?4,0,NULL) ON CONFLICT(source_key) DO UPDATE SET policy_json=excluded.policy_json,generation=generation+withdrawn,withdrawn=0",params![request.policy.source_key,caller.actor,request.policy.scope,serde_json::to_string(&request.policy)?])?;
        tx.execute(
            "UPDATE system_meta SET policy_epoch=policy_epoch+1 WHERE singleton=1",
            [],
        )?;
        audit(&tx, t, caller, "admit_source", &request.policy.source_key)?;
        caller.check()?;
        tx.commit()?;
        Ok(request.policy)
    }
    pub fn source_generation(&self, caller: &Caller, source_key: &str) -> Result<u64> {
        let visible=self.conn.prepare("SELECT 1 FROM sources WHERE source_key=?1 AND scope IN(SELECT value FROM json_each(?2)) AND EXISTS(SELECT 1 FROM json_each(policy_json,'$.allowed_actors') WHERE value=?3)")?.exists(params![source_key,serde_json::to_string(&caller.read_scopes)?,caller.actor])?;
        require(visible, "NOT_FOUND", "No authorized source matches")?;
        let (policy, generation) = source_policy(&self.conn, source_key)?;
        require(
            caller.read(&policy.scope) && policy.allowed_actors.contains(&caller.actor),
            "PERMISSION_DENIED",
            "Source is outside caller admission",
        )?;
        Ok(generation)
    }
    pub(crate) fn publish_source(
        &mut self,
        caller: &Caller,
        batch: SourceBatch,
    ) -> Result<ImportResult> {
        caller.require("ingest")?;
        caller.check()?;
        batch.data.validate_bounds()?;
        text(&batch.capture.revision, 256)?;
        identifier(&batch.scope_key)?;
        require(
            batch.raw_proposals.len() <= 1000,
            "LIMIT_EXCEEDED",
            "Too many proposals",
        )?;
        // Only request inputs identify a retry: a model can extract different data from the same
        // capture. The generation a retry reads afresh is not one of them. Earlier builds hashed
        // the extracted data instead of the revision.
        let request_hash = stable_id(
            "import",
            &(
                &batch.source_key,
                &batch.scope_key,
                batch.complete,
                &batch.capture.sha256,
                &batch.capture.revision,
                &batch.extractor_digest,
            ),
        )?;
        let legacy_hash = stable_id(
            "import",
            &(
                &batch.source_key,
                &batch.scope_key,
                batch.complete,
                &batch.capture.sha256,
                &batch.extractor_digest,
                &batch.data,
            ),
        )?;
        let tx = self.begin(caller, batch.policy_epoch)?;
        if let Some(result) = replay::<ImportResult>(
            &tx,
            caller,
            &batch.request_key,
            &[&request_hash, &legacy_hash],
        )? {
            tx.commit()?;
            return Ok(result);
        }
        if let Some(id) = &batch.job_id {
            require(tx.prepare("SELECT 1 FROM jobs WHERE job_id=?1 AND actor=?2 AND state='running' AND policy_epoch=?3")?.exists(params![id,caller.actor,batch.policy_epoch])?,"CANCELLED","Job is no longer eligible for publication")?;
        }
        // Provider capture authority is checked here again, not inherited from successful HTTP.
        if batch.capture.provider_id.is_some() {
            crate::acquisition::providers::authorize_capture(&tx, caller, &batch)?;
        } else {
            require(
                batch.provider_properties.is_empty(),
                "PERMISSION_DENIED",
                "Only provider captures can register provider types",
            )?;
        }
        let (policy, generation) = source_policy(&tx, &batch.source_key)?;
        require(
            caller.write(&policy.scope) && policy.allowed_actors.contains(&caller.actor),
            "PERMISSION_DENIED",
            "Source publication is not authorized",
        )?;
        // A source keeps the capture origin of its history: a provider-owned source accepts only
        // that provider's captures, so a local import cannot publish or withdraw under its policy.
        let prior_origin: Option<Option<String>> = tx
            .query_row(
                "SELECT json_extract(capture_json,'$.provider_id') FROM source_revisions WHERE source_key=?1 ORDER BY generation DESC LIMIT 1",
                [&batch.source_key],
                |r| r.get(0),
            )
            .optional()?;
        require(
            prior_origin.is_none_or(|origin| origin == batch.capture.provider_id),
            "POLICY_DENIED",
            "Source is owned by another capture origin",
        )?;
        require(
            generation == batch.expected_generation,
            "VERSION_CONFLICT",
            "Source generation changed",
        )?;
        let prior_hash: Option<String> = tx
            .query_row(
                "SELECT sha256 FROM source_revisions WHERE source_key=?1 AND revision=?2 LIMIT 1",
                params![batch.source_key, batch.capture.revision],
                |r| r.get(0),
            )
            .optional()?;
        require(
            prior_hash
                .as_ref()
                .is_none_or(|h| h == &batch.capture.sha256),
            "VERSION_CONFLICT",
            "An immutable source revision cannot change content",
        )?;
        let t = tick(&tx)?;
        let next = generation + 1;
        tx.execute("INSERT INTO source_revisions(source_key,generation,revision,sha256,capture_json,tx_tick) VALUES(?1,?2,?3,?4,?5,?6)",params![batch.source_key,next,batch.capture.revision,batch.capture.sha256,serde_json::to_string(&batch.capture)?,t])?;
        for entity in &batch.data.entities {
            require(
                policy.entity_namespaces.contains(&entity.namespace),
                "PERMISSION_DENIED",
                "Source cannot publish into this entity namespace",
            )?;
            insert_entity(&tx, caller, &policy.scope, entity, t)?;
        }
        let active = if batch.semantic {
            policy.auto_admit_semantic
        } else {
            policy.auto_admit_structured
        };
        let mut retained = BTreeSet::new();
        let mut affected = BTreeSet::new();
        let mut result = ImportResult {
            source_key: batch.source_key.clone(),
            generation: next,
            published: Vec::new(),
            proposal_ids: Vec::new(),
            withdrawn_assertion_ids: Vec::new(),
        };
        for assertion in &batch.data.assertions {
            caller.check()?;
            let normalized = normalize_relation(&tx, caller, &assertion.relation, &policy.scope);
            // Admission compares the registered key, as review does, not the input spelling.
            let admitted = policy.allows_type(
                normalized
                    .as_ref()
                    .map_or(&assertion.relation.relation_type, |(r, _)| &r.relation_type),
            );
            if !admitted || normalized.as_ref().is_err_and(|e| e.code == "TYPE_UNKNOWN") {
                let reason = if !admitted {
                    "type_not_admitted"
                } else {
                    "type_unknown"
                };
                result.proposal_ids.push(insert_proposal(
                    &tx,
                    caller,
                    &policy.scope,
                    Some(&batch.source_key),
                    reason,
                    &serde_json::to_value(assertion)?,
                    t,
                )?);
                continue;
            }
            let (relation, relation_id) = normalized?;
            validate_evidence(&tx, caller, &policy.scope, &assertion.evidence)?;
            let assertion_id = stable_id(
                "assert",
                &(
                    &batch.source_key,
                    &assertion.anchor,
                    &relation_id,
                    &batch.extractor_digest,
                ),
            )?;
            let previous_scope: Option<String> = tx
                .query_row(
                    "SELECT extraction_scope_key FROM relation_assertions WHERE assertion_id=?1",
                    [&assertion_id],
                    |r| r.get(0),
                )
                .optional()?;
            require(
                previous_scope
                    .as_ref()
                    .is_none_or(|s| s == &batch.scope_key),
                "VERSION_CONFLICT",
                "An assertion anchor cannot silently move to another reconciliation scope",
            )?;
            // Turning off auto-admission does not revoke a previously accepted assertion.
            // Keep that assertion (freshness is independently evaluated), and retain the
            // unapproved refresh as a proposal rather than downgrading the active head. A
            // reviewer's rejection is durable the same way: a refresh goes back to review.
            let current: Option<String> = tx
                .query_row(
                    "SELECT current_status FROM relation_assertions WHERE assertion_id=?1",
                    [&assertion_id],
                    |r| r.get(0),
                )
                .optional()?;
            let current = current.as_deref();
            if current == Some("rejected") || (!active && current == Some("active")) {
                result.proposal_ids.push(insert_proposal(
                    &tx,
                    caller,
                    &policy.scope,
                    Some(&batch.source_key),
                    "refresh_requires_review",
                    &serde_json::to_value(assertion)?,
                    t,
                )?);
                retained.insert(assertion_id);
                continue;
            }
            insert_relation(&tx, &relation_id, &relation, t)?;
            let origin = Origin {
                kind: batch.origin_kind.clone(),
                actor: caller.actor.clone(),
                operation_id: String::new(),
                source_key: Some(batch.source_key.clone()),
                source_revision: Some(batch.capture.revision.clone()),
                source_generation: Some(next),
                extraction_scope_key: Some(batch.scope_key.clone()),
                extractor_digest: Some(batch.extractor_digest.clone()),
            };
            let epistemic =
                if batch.semantic && assertion.epistemic_kind != EpistemicKind::Hypothetical {
                    EpistemicKind::Extracted
                } else {
                    assertion.epistemic_kind.clone()
                };
            let (version, changed) = append_assertion(
                &tx,
                caller,
                AssertionWrite {
                    assertion_id: &assertion_id,
                    relation_id: &relation_id,
                    scope: &policy.scope,
                    anchor: &assertion.anchor,
                    status: if active {
                        PublicationStatus::Active
                    } else {
                        PublicationStatus::Proposed
                    },
                    epistemic,
                    origin,
                    evidence: assertion.evidence.clone(),
                    tick: t,
                },
            )?;
            retained.insert(assertion_id.clone());
            affected.insert(relation_id.clone());
            result.published.push(Published {
                relation_id,
                assertion_id,
                assertion_version: version,
                publication_status: if active {
                    PublicationStatus::Active
                } else {
                    PublicationStatus::Proposed
                },
                changed,
            });
        }
        for (reason, payload) in &batch.raw_proposals {
            text(reason, 256)?;
            result.proposal_ids.push(insert_proposal(
                &tx,
                caller,
                &policy.scope,
                Some(&batch.source_key),
                reason,
                payload,
                t,
            )?);
        }
        // A proposal/unsupported row means the scope was not successfully enumerated.
        let complete = batch.complete && active && result.proposal_ids.is_empty();
        if complete {
            let mut statement=tx.prepare("SELECT assertion_id,relation_id FROM relation_assertions WHERE source_key=?1 AND extraction_scope_key=?2 AND current_status IN ('active','proposed')")?;
            let old = statement
                .query_map(params![batch.source_key, batch.scope_key], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            drop(statement);
            for (assertion_id, relation_id) in old {
                if !retained.contains(&assertion_id) {
                    withdraw(&tx, caller, &assertion_id, t)?;
                    affected.insert(relation_id);
                    result.withdrawn_assertion_ids.push(assertion_id);
                }
            }
        }
        tx.execute("UPDATE sources SET generation=?1,current_revision=?2,withdrawn=0 WHERE source_key=?3 AND generation=?4",params![next,batch.capture.revision,batch.source_key,generation])?;
        tx.execute("INSERT INTO source_extraction_scopes(source_key,scope_key,generation,revision,complete,extractor_digest) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(source_key,scope_key) DO UPDATE SET generation=excluded.generation,revision=excluded.revision,complete=excluded.complete,extractor_digest=excluded.extractor_digest",params![batch.source_key,batch.scope_key,next,batch.capture.revision,complete,batch.extractor_digest])?;
        for id in affected {
            snapshot_relation(&tx, &id, t)?;
        }
        audit(&tx, t, caller, "publish_source", &batch.source_key)?;
        remember(&tx, caller, &batch.request_key, &request_hash, &result)?;
        if let Some(job) = &batch.job_id {
            let prior: String =
                tx.query_row("SELECT report_json FROM jobs WHERE job_id=?1", [job], |r| {
                    r.get(0)
                })?;
            let mut report: serde_json::Value = serde_json::from_str(&prior)?;
            if report.get("committed_sources").is_none() {
                report["committed_sources"] = serde_json::json!([]);
            }
            let list = report["committed_sources"]
                .as_array_mut()
                .ok_or_else(|| Error::invalid("Invalid internal job report"))?;
            require(
                list.len() < 16,
                "LIMIT_EXCEEDED",
                "Job publication bound exceeded",
            )?;
            list.push(serde_json::json!({"source_key":result.source_key,"generation":result.generation,"active_assertions":result.published.iter().filter(|p|p.publication_status==PublicationStatus::Active).count(),"proposed_assertions":result.published.iter().filter(|p|p.publication_status==PublicationStatus::Proposed).count(),"raw_proposals":result.proposal_ids.len(),"withdrawals":result.withdrawn_assertion_ids.len()}));
            tx.execute(
                "UPDATE jobs SET report_json=?2,version=version+1,updated_at=?3 WHERE job_id=?1",
                params![job, serde_json::to_string(&report)?, now_ms()],
            )?;
        }
        caller.check()?;
        tx.commit()?;
        Ok(result)
    }
    pub fn withdraw_source(
        &mut self,
        caller: &Caller,
        request: WithdrawSourceRequest,
    ) -> Result<Vec<String>> {
        caller.require("write")?;
        let tx = self.begin(caller, request.expected_policy_epoch)?;
        let visible=tx.prepare("SELECT 1 FROM sources WHERE source_key=?1 AND owner=?2 AND scope IN(SELECT value FROM json_each(?3))")?.exists(params![request.source_key,caller.actor,serde_json::to_string(&caller.read_scopes)?])?;
        require(visible, "NOT_FOUND", "No owned source matches")?;
        let (policy, generation) = source_policy(&tx, &request.source_key)?;
        let owner: String = tx.query_row(
            "SELECT owner FROM sources WHERE source_key=?1",
            [&request.source_key],
            |r| r.get(0),
        )?;
        require(
            owner == caller.actor && caller.write(&policy.scope),
            "PERMISSION_DENIED",
            "Only the source owner can withdraw this source",
        )?;
        require(
            generation == request.expected_generation,
            "VERSION_CONFLICT",
            "Source generation changed",
        )?;
        let t = tick(&tx)?;
        let mut stmt=tx.prepare("SELECT assertion_id,relation_id FROM relation_assertions WHERE source_key=?1 AND current_status IN ('active','proposed')")?;
        let assertions = stmt
            .query_map([&request.source_key], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        let mut affected = BTreeSet::new();
        let mut ids = Vec::new();
        for (id, relation) in assertions {
            withdraw(&tx, caller, &id, t)?;
            affected.insert(relation);
            ids.push(id);
        }
        for relation in affected {
            snapshot_relation(&tx, &relation, t)?;
        }
        tx.execute(
            "UPDATE sources SET withdrawn=1,generation=generation+1 WHERE source_key=?1",
            [&request.source_key],
        )?;
        tx.execute(
            "UPDATE system_meta SET policy_epoch=policy_epoch+1 WHERE singleton=1",
            [],
        )?;
        audit(&tx, t, caller, "withdraw_source", &request.source_key)?;
        caller.check()?;
        tx.commit()?;
        Ok(ids)
    }
}

pub(crate) fn source_policy(conn: &Connection, key: &str) -> Result<(SourcePolicy, u64)> {
    let data: Option<(String, u64, bool)> = conn
        .query_row(
            "SELECT policy_json,generation,withdrawn FROM sources WHERE source_key=?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (policy, generation, withdrawn) = data.ok_or_else(Error::not_found)?;
    require(
        !withdrawn,
        "PERMISSION_DENIED",
        "Source is withdrawn; a new explicit admission is required",
    )?;
    Ok((serde_json::from_str(&policy)?, generation))
}
pub(crate) fn insert_entity(
    conn: &Connection,
    caller: &Caller,
    scope: &str,
    input: &EntityInput,
    t: u64,
) -> Result<String> {
    input.validate()?;
    require(
        caller.write(scope),
        "PERMISSION_DENIED",
        "Entity scope is not writable",
    )?;
    let id = input.id()?;
    conn.execute("INSERT OR IGNORE INTO entities(entity_id,namespace,kind,canonical_key,created_tick) VALUES(?1,?2,?3,?4,?5)",params![id,input.namespace,input.kind,input.canonical_key,t])?;
    for label in std::iter::once(&input.name).chain(input.aliases.iter()) {
        conn.execute("INSERT OR IGNORE INTO entity_labels(entity_id,scope,label,actor,tx_tick) VALUES(?1,?2,?3,?4,?5)",params![id,scope,label,caller.actor,t])?;
    }
    Ok(id)
}
fn insert_relation(conn: &Connection, id: &str, input: &RelationInput, t: u64) -> Result<()> {
    let ty = load_type(conn, &input.relation_type)?;
    let inserted=conn.execute("INSERT OR IGNORE INTO relations(relation_id,canonical_key,canonicalizer_version,type_name,type_version,relation_json,created_tick) VALUES(?1,?1,1,?2,?3,?4,?5)",params![id,ty.name,ty.version,serde_json::to_string(input)?,t])?;
    if inserted == 1 {
        for (role, values) in &input.roles {
            for (ordinal, value) in values.iter().enumerate() {
                let (entity, literal) = match value {
                    RoleValue::Entity { entity_id } => (Some(entity_id.as_str()), None),
                    RoleValue::Literal { literal } => (None, Some(serde_json::to_string(literal)?)),
                };
                conn.execute("INSERT INTO relation_role_bindings(relation_id,role,ordinal,entity_id,literal_json) VALUES(?1,?2,?3,?4,?5)",params![id,role,ordinal as u64,entity,literal])?;
            }
        }
    }
    Ok(())
}
fn validate_evidence(
    conn: &Connection,
    caller: &Caller,
    scope: &str,
    evidence: &[Evidence],
) -> Result<()> {
    require(
        evidence.len() <= 16,
        "LIMIT_EXCEEDED",
        "Too many evidence references",
    )?;
    for item in evidence {
        text(&item.source_revision, 512)?;
        text(&item.locator, 2048)?;
        require(
            item.sha256.len() == 64
                && item
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "INVALID_INPUT",
            "Evidence needs a lowercase SHA-256",
        )?;
        require(caller.read(scope) && conn.prepare("SELECT 1 FROM source_revisions r JOIN sources s USING(source_key) WHERE r.source_key||'@'||r.generation=?1 AND r.sha256=?2 AND s.scope=?3")?.exists(params![item.source_revision,item.sha256,scope])?,"INVALID_INPUT","Evidence revision/hash is unavailable in this scope")?;
    }
    Ok(())
}
struct AssertionWrite<'a> {
    assertion_id: &'a str,
    relation_id: &'a str,
    scope: &'a str,
    anchor: &'a str,
    status: PublicationStatus,
    epistemic: EpistemicKind,
    origin: Origin,
    evidence: Vec<Evidence>,
    tick: u64,
}
fn append_assertion(
    conn: &Connection,
    caller: &Caller,
    mut write: AssertionWrite<'_>,
) -> Result<(u64, bool)> {
    require(
        write.epistemic != EpistemicKind::Inferred,
        "INFERENCE_DISABLED",
        "Persistent inference is disabled",
    )?;
    let previous:Option<(u64,String,String,String,String)>=conn.query_row("SELECT a.current_version,v.status,v.epistemic_kind,v.origin_json,v.evidence_json FROM relation_assertions a JOIN assertion_versions v ON v.assertion_id=a.assertion_id AND v.version=a.current_version WHERE a.assertion_id=?1",[write.assertion_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let status = enum_string(&write.status)?;
    let epistemic = enum_string(&write.epistemic)?;
    let version = if let Some((
        version,
        prior_status,
        prior_epistemic,
        prior_origin,
        prior_evidence,
    )) = previous
    {
        let prior: Origin = serde_json::from_str(&prior_origin)?;
        if prior_status == status
            && prior_epistemic == epistemic
            && prior_evidence == serde_json::to_string(&write.evidence)?
            && prior.kind == write.origin.kind
            && prior.source_revision == write.origin.source_revision
            && prior.extraction_scope_key == write.origin.extraction_scope_key
            && prior.extractor_digest == write.origin.extractor_digest
        {
            return Ok((version, false));
        }
        conn.execute("UPDATE relation_assertions SET current_version=current_version+1,current_status=?1 WHERE assertion_id=?2 AND current_version=?3",params![status,write.assertion_id,version])?;
        version + 1
    } else {
        conn.execute("INSERT INTO relation_assertions(assertion_id,relation_id,actor,scope,source_key,origin_assertion_key,extraction_scope_key,extractor_digest,current_version,current_status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,?9)",params![write.assertion_id,write.relation_id,caller.actor,write.scope,write.origin.source_key,write.anchor,write.origin.extraction_scope_key,write.origin.extractor_digest,status])?;
        1
    };
    write.origin.operation_id = stable_id(
        "op",
        &(
            write.tick,
            &caller.actor,
            "assertion_version",
            write.assertion_id,
        ),
    )?;
    let id = stable_id("av", &(write.assertion_id, version))?;
    conn.execute("INSERT INTO assertion_versions(assertion_version_id,assertion_id,version,tx_tick,status,epistemic_kind,origin_json,evidence_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,write.assertion_id,version,write.tick,status,epistemic,serde_json::to_string(&write.origin)?,serde_json::to_string(&write.evidence)?])?;
    audit(
        conn,
        write.tick,
        caller,
        "assertion_version",
        write.assertion_id,
    )?;
    Ok((version, true))
}
fn withdraw(conn: &Connection, caller: &Caller, id: &str, t: u64) -> Result<u64> {
    let (relation,scope,anchor,epistemic,origin,evidence):(String,String,String,String,String,String)=conn.query_row("SELECT a.relation_id,a.scope,a.origin_assertion_key,v.epistemic_kind,v.origin_json,v.evidence_json FROM relation_assertions a JOIN assertion_versions v ON v.assertion_id=a.assertion_id AND v.version=a.current_version WHERE a.assertion_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    let epistemic: EpistemicKind = serde_json::from_value(serde_json::Value::String(epistemic))?;
    Ok(append_assertion(
        conn,
        caller,
        AssertionWrite {
            assertion_id: id,
            relation_id: &relation,
            scope: &scope,
            anchor: &anchor,
            status: PublicationStatus::Withdrawn,
            epistemic,
            origin: serde_json::from_str(&origin)?,
            evidence: serde_json::from_str(&evidence)?,
            tick: t,
        },
    )?
    .0)
}
fn snapshot_relation(conn: &Connection, id: &str, t: u64) -> Result<()> {
    let mut stmt=conn.prepare("SELECT v.assertion_version_id FROM relation_assertions a JOIN assertion_versions v ON v.assertion_id=a.assertion_id AND v.version=a.current_version WHERE a.relation_id=?1 AND a.current_status='active' ORDER BY v.assertion_version_id")?;
    let ids = stmt
        .query_map([id], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(stmt);
    let hash = stable_id("set", &ids)?;
    let previous:Option<String>=conn.query_row("SELECT active_set_hash FROM relation_versions WHERE relation_id=?1 ORDER BY tx_tick DESC LIMIT 1",[id],|r|r.get(0)).optional()?;
    if previous.as_ref() == Some(&hash) {
        return Ok(());
    }
    let version_id = stable_id("rv", &(id, t, &hash))?;
    conn.execute("INSERT INTO relation_versions(relation_version_id,relation_id,tx_tick,active_set_hash) VALUES(?1,?2,?3,?4)",params![version_id,id,t,hash])?;
    for assertion in ids {
        conn.execute(
            "INSERT INTO relation_version_assertions VALUES(?1,?2)",
            params![version_id, assertion],
        )?;
    }
    Ok(())
}
pub(crate) fn insert_proposal(
    conn: &Connection,
    caller: &Caller,
    scope: &str,
    source: Option<&str>,
    reason: &str,
    payload: &serde_json::Value,
    t: u64,
) -> Result<String> {
    let id = stable_id("proposal", &(&caller.actor, scope, source, reason, payload))?;
    conn.execute("INSERT OR IGNORE INTO relation_proposals(proposal_id,actor,scope,source_key,reason,payload_json,tx_tick) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,caller.actor,scope,source,reason,serde_json::to_string(payload)?,t])?;
    Ok(id)
}
fn enum_string<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::invalid("Invalid enum value"))
}
fn replay<T: serde::de::DeserializeOwned>(
    conn: &Connection,
    caller: &Caller,
    key: &str,
    hashes: &[&str],
) -> Result<Option<T>> {
    let prior:Option<(String,String)>=conn.query_row("SELECT request_hash,response_json FROM write_requests WHERE actor=?1 AND request_key=?2",params![caller.actor,key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((old_hash, body)) = prior {
        require(
            hashes.contains(&old_hash.as_str()),
            "VERSION_CONFLICT",
            "Idempotency key was already used for a different request",
        )?;
        Ok(Some(serde_json::from_str(&body)?))
    } else {
        Ok(None)
    }
}
fn remember<T: serde::Serialize>(
    conn: &Connection,
    caller: &Caller,
    key: &str,
    hash: &str,
    result: &T,
) -> Result<()> {
    conn.execute("INSERT INTO write_requests(actor,request_key,request_hash,response_json) VALUES(?1,?2,?3,?4)",params![caller.actor,key,hash,serde_json::to_string(result)?])?;
    Ok(())
}

/// relation_id, actor, scope, anchor, current_version, current_status, epistemic_kind, origin_json, evidence_json
type ReviewRow = (
    String,
    String,
    String,
    String,
    u64,
    String,
    String,
    String,
    String,
);
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewAssertionRequest {
    pub assertion_id: String,
    pub expected_version: u64,
    pub expected_policy_epoch: u64,
    pub approve: bool,
}
impl Store {
    /// An explicit human review changes a proposal's disposition, not its source provenance.
    pub fn review_assertion(
        &mut self,
        caller: &Caller,
        request: ReviewAssertionRequest,
    ) -> Result<Published> {
        caller.require("review")?;
        let tx = self.begin(caller, request.expected_policy_epoch)?;
        let row:Option<ReviewRow>=tx.query_row("SELECT a.relation_id,a.actor,a.scope,a.origin_assertion_key,a.current_version,a.current_status,v.epistemic_kind,v.origin_json,v.evidence_json FROM relation_assertions a JOIN assertion_versions v ON v.assertion_id=a.assertion_id AND v.version=a.current_version WHERE a.assertion_id=?1 AND a.actor=?2 AND a.scope IN(SELECT value FROM json_each(?3))",params![request.assertion_id,caller.actor,serde_json::to_string(&caller.read_scopes)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional()?;
        let (relation_id, actor, scope, anchor, version, status, epistemic, origin, evidence) =
            row.ok_or_else(Error::not_found)?;
        require(
            actor == caller.actor && caller.write(&scope),
            "PERMISSION_DENIED",
            "Review is restricted to the caller's admitted assertions",
        )?;
        require(
            status == "proposed" && version == request.expected_version,
            "VERSION_CONFLICT",
            "Assertion is not the expected proposal version",
        )?;
        let origin: Origin = serde_json::from_str(&origin)?;
        if let Some(source) = &origin.source_key {
            let (policy, _) = source_policy(&tx, source)?;
            let type_key: String = tx.query_row(
                "SELECT type_name || '@' || type_version FROM relations WHERE relation_id=?1",
                [&relation_id],
                |r| r.get(0),
            )?;
            require(
                policy.allowed_actors.contains(&caller.actor)
                    && policy.scope == scope
                    && policy.allows_type(&type_key),
                "POLICY_DENIED",
                "Current source admission does not authorize this review",
            )?;
        }
        let t = tick(&tx)?;
        let publication_status = if request.approve {
            PublicationStatus::Active
        } else {
            PublicationStatus::Rejected
        };
        let (version, changed) = append_assertion(
            &tx,
            caller,
            AssertionWrite {
                assertion_id: &request.assertion_id,
                relation_id: &relation_id,
                scope: &scope,
                anchor: &anchor,
                status: publication_status.clone(),
                epistemic: serde_json::from_value(serde_json::Value::String(epistemic))?,
                origin,
                evidence: serde_json::from_str(&evidence)?,
                tick: t,
            },
        )?;
        snapshot_relation(&tx, &relation_id, t)?;
        caller.check()?;
        tx.commit()?;
        Ok(Published {
            relation_id,
            assertion_id: request.assertion_id,
            assertion_version: version,
            publication_status,
            changed,
        })
    }
}
