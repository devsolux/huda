//! The sole acquisition controller: consume typed needs, schedule bounded reads, re-query.
//! It never interprets user prose, evaluates slot coverage or publishes outside Store.
use super::{
    admission::*,
    providers::{self, wikimedia::Budget},
    structured,
};
use crate::api::App;
use crate::error::{require, Error, Result};
use crate::relation::{self, now_ms, query::RelationContext, Caller};
use crate::storage::{audit, tick, Store};
use rusqlite::{params, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobView {
    pub job_id: String,
    pub kind: String,
    pub state: String,
    pub goal: Value,
    pub report: Value,
    pub version: u64,
    pub created_at: u64,
    pub updated_at: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    pub job_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelJobRequest {
    pub job_id: String,
    pub expected_version: u64,
}
/// kind, state, goal_json, report_json, scope, version, created_at, updated_at
type JobRow = (String, String, String, String, String, u64, u64, u64);
impl Store {
    pub(crate) fn create_job(
        &mut self,
        caller: &Caller,
        kind: &str,
        scope: &str,
        goal: &Value,
        epoch: u64,
    ) -> Result<JobView> {
        caller.require("jobs")?;
        require(
            caller.write(scope),
            "PERMISSION_DENIED",
            "Job scope is not writable",
        )?;
        require(
            relation::canonical_bytes(goal)?.len() <= 16_384,
            "LIMIT_EXCEEDED",
            "Job parameters are too large",
        )?;
        let tx = self.begin(caller, epoch)?;
        let pending: u64 = tx.query_row(
            "SELECT count(*) FROM jobs WHERE state IN ('queued','running')",
            [],
            |r| r.get(0),
        )?;
        require(pending < 4, "BACKPRESSURE", "Core job capacity is full")?;
        let id = relation::random_id("job")?;
        let now = now_ms();
        let t = tick(&tx)?;
        tx.execute("INSERT INTO jobs(job_id,actor,scope,kind,state,goal_json,policy_epoch,report_json,version,created_at,updated_at) VALUES(?1,?2,?3,?4,'queued',?5,?6,'{}',1,?7,?7)",params![id,caller.actor,scope,kind,serde_json::to_string(goal)?,epoch,now])?;
        audit(&tx, t, caller, "create_job", &id)?;
        caller.check()?;
        tx.commit()?;
        self.get_job(caller, &id)
    }
    pub fn get_job(&self, caller: &Caller, id: &str) -> Result<JobView> {
        caller.require("jobs")?;
        caller.check()?;
        let row:Option<JobRow>=self.conn.query_row("SELECT kind,state,goal_json,report_json,scope,version,created_at,updated_at FROM jobs WHERE job_id=?1 AND actor=?2",params![id,caller.actor],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?;
        let (kind, state, goal, report, scope, version, created_at, updated_at) =
            row.ok_or_else(Error::not_found)?;
        require(caller.read(&scope), "NOT_FOUND", "Job is not visible")?;
        Ok(JobView {
            job_id: id.into(),
            kind,
            state,
            goal: serde_json::from_str(&goal)?,
            report: serde_json::from_str(&report)?,
            version,
            created_at,
            updated_at,
        })
    }
    pub(crate) fn begin_job(&mut self, caller: &Caller, id: &str, epoch: u64) -> Result<()> {
        let tx = self.begin(caller, epoch)?;
        require(tx.execute("UPDATE jobs SET state='running',version=version+1,updated_at=?3 WHERE job_id=?1 AND actor=?2 AND state='queued' AND policy_epoch=?4",params![id,caller.actor,now_ms(),epoch])?==1,"CANCELLED","Job is no longer queued")?;
        caller.check()?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn finish_job(
        &mut self,
        caller: &Caller,
        id: &str,
        result: &Result<Value>,
    ) -> Result<()> {
        // Completion of an already authorized job is operational state, not semantic publication.
        // Policy revocation/cancellation must still be recordable after the job token expires.
        let (state, mut report) = match result {
            Ok(report) => (
                if report
                    .get("stop_reason")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !matches!(s, "goal_met" | "ingested" | "captured"))
                {
                    "partial"
                } else {
                    "completed"
                },
                report.clone(),
            ),
            Err(error) => (
                match error.code.as_str() {
                    "CANCELLED" => "cancelled",
                    "POLICY_CHANGED" => "superseded",
                    "OFFLINE" | "POLICY_DENIED" | "PERMISSION_DENIED" => "blocked",
                    _ => "failed",
                },
                json!({"stop_reason":error.code.to_lowercase(),"error":error}),
            ),
        };
        let prior: Option<String> = self
            .conn
            .query_row(
                "SELECT report_json FROM jobs WHERE job_id=?1 AND actor=?2",
                params![id, caller.actor],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(prior) = prior {
            if let Some(committed) = serde_json::from_str::<Value>(&prior)?.get("committed_sources")
            {
                report["committed_sources"] = committed.clone();
            }
        }
        self.conn.execute("UPDATE jobs SET state=?3,report_json=?4,version=version+1,updated_at=?5 WHERE job_id=?1 AND actor=?2 AND state IN ('queued','running')",params![id,caller.actor,state,serde_json::to_string(&report)?,now_ms()])?;
        Ok(())
    }
    pub fn cancel_job(&mut self, caller: &Caller, request: CancelJobRequest) -> Result<JobView> {
        let current = self.get_job(caller, &request.job_id)?;
        require(
            current.version == request.expected_version,
            "VERSION_CONFLICT",
            "Job version changed",
        )?;
        require(
            matches!(current.state.as_str(), "queued" | "running"),
            "VERSION_CONFLICT",
            "Job is already terminal",
        )?;
        // Cancellation never implies an earlier atomic commit was undone: keep committed_sources.
        self.conn.execute("UPDATE jobs SET state='cancelled',report_json=json_set(report_json,'$.stop_reason','cancelled'),version=version+1,updated_at=?3 WHERE job_id=?1 AND actor=?2 AND version=?4",params![request.job_id,caller.actor,now_ms(),request.expected_version])?;
        self.get_job(caller, &request.job_id)
    }
}

pub(crate) fn eligible(
    app: &App,
    caller: &Caller,
    context: &RelationContext,
    epoch: u64,
) -> Result<Vec<(String, String, String)>> {
    caller.require("expand")?;
    if context.goal_coverage.goal_met || context.knowledge_needs.is_empty() {
        return Ok(Vec::new());
    }
    let store = app.store()?;
    store.check_policy(caller, epoch)?;
    if store.epochs()?.offline {
        return Ok(Vec::new());
    }
    let needed: BTreeSet<_> = context
        .knowledge_needs
        .iter()
        .flat_map(|n| n.seed_ids.iter())
        .collect();
    let mut reads = BTreeSet::new();
    for provider in ["wikidata", "wikipedia"] {
        let Ok(policy) = providers::policy(&store.conn, provider) else {
            continue;
        };
        if !policy.may_fetch
            || !policy.languages.contains(&context.goal.language)
            || !policy.allowed_actors.contains(&caller.actor)
            || !caller.write(&policy.scope)
        {
            continue;
        }
        for seed in &context.goal.seeds {
            if needed.contains(&seed.seed_id) {
                // Canonical approved spelling is sent; no raw question or model-chosen URL.
                if let Some(term) = policy
                    .approved_public_terms
                    .iter()
                    .find(|term| term.eq_ignore_ascii_case(&seed.label))
                {
                    reads.insert((provider.to_string(), term.clone(), policy.scope.clone()));
                }
            }
        }
    }
    Ok(reads.into_iter().collect())
}

pub(crate) fn run(
    app: App,
    caller: Caller,
    job_id: String,
    mut context: RelationContext,
    epoch: u64,
) -> Result<Value> {
    let original_goal = context.goal.clone();
    let mut budget = Budget::new();
    let mut seen = BTreeSet::new();
    let mut captures = Vec::new();
    let mut proposals = 0usize;
    let mut used = BTreeSet::new();
    let before: BTreeSet<_> = context
        .relations
        .iter()
        .map(|r| r.relation_id.clone())
        .collect();
    let mut items = 0;
    let mut articles = 0;
    let mut stop = "no_match";
    'rounds: for _round in 0..2 {
        let reads = eligible(&app, &caller, &context, epoch)?;
        if reads.is_empty() {
            stop = if context.goal_coverage.goal_met {
                "goal_met"
            } else {
                "denied"
            };
            break;
        }
        let mut made_progress = false;
        for (provider, term, scope) in reads {
            caller.check()?;
            if budget.check().is_err() {
                stop = "exhausted";
                break;
            }
            if !seen.insert((provider.clone(), term.clone())) {
                continue;
            }
            // Article text is only publishable through local extraction; spend no egress or
            // budget on pages that would be discarded without a model.
            if provider == "wikipedia" && !app.model.available() {
                stop = "model_unavailable";
                continue;
            }
            if (provider == "wikidata" && items >= 5) || (provider == "wikipedia" && articles >= 3)
            {
                stop = "exhausted";
                continue;
            }
            // Start a term only when its worst case fits: search, fetch and, for Wikidata, one
            // property-metadata read. A half-read term would only spend egress.
            if !budget.has_room(if provider == "wikidata" { 3 } else { 2 }) {
                stop = "exhausted";
                break 'rounds;
            }
            let language = &original_goal.language;
            let checkpoint = || {
                let store = app.store()?;
                providers::check_egress(&store.conn, &caller, &provider, language, &term, epoch)
                    .map(|_| ())
            };
            let policy = {
                let store = app.store()?;
                providers::check_egress(&store.conn, &caller, &provider, language, &term, epoch)?
            };
            let candidates =
                match app
                    .wikimedia
                    .discover(&policy, language, &term, &mut budget, &checkpoint)
                {
                    Ok(candidates) => candidates,
                    Err(error) => {
                        stop = soft_stop(&error).ok_or(error)?;
                        break 'rounds;
                    }
                };
            // Never resolve an ambiguous label by taking the first search hit.
            let exact: Vec<_> = candidates
                .into_iter()
                .filter(|c| c.label.eq_ignore_ascii_case(&term))
                .collect();
            if exact.len() != 1 {
                stop = if exact.is_empty() {
                    "no_match"
                } else {
                    "ambiguous"
                };
                continue;
            }
            let candidate = &exact[0];
            let source_key = relation::stable_id(
                "src",
                &(&provider, language, &candidate.id, &caller.actor, &scope),
            )?;
            // Only fresh immutable source captures may short-circuit a repeated fetch.
            let cached = {
                let store = app.store()?;
                store.conn.query_row("SELECT capture_json FROM source_revisions WHERE source_key=?1 ORDER BY generation DESC LIMIT 1",[&source_key],|r|r.get::<_,String>(0)).optional()?
            };
            let cached_capture = cached
                .map(|value| serde_json::from_str::<Capture>(&value))
                .transpose()?
                .filter(|capture| capture.expires_at.is_some_and(|expires| expires > now_ms()));
            let capture = match cached_capture {
                Some(capture) => capture,
                None => match app.wikimedia.fetch(
                    &policy,
                    language,
                    candidate,
                    &mut budget,
                    &checkpoint,
                ) {
                    Ok(capture) => capture,
                    Err(error) => {
                        stop = soft_stop(&error).ok_or(error)?;
                        break 'rounds;
                    }
                },
            };
            if provider == "wikidata" {
                items += 1;
            } else {
                articles += 1;
            }
            used.insert(provider.clone());
            checkpoint()?;
            let (data, properties, raw_proposals, semantic) = if provider == "wikidata" {
                let pids: Vec<String> = capture
                    .payload
                    .get("claims")
                    .and_then(Value::as_object)
                    .map(|m| {
                        m.keys()
                            .filter(|id| providers::wikimedia::external_id(id, b'P'))
                            .take(16)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                let properties = match app.wikimedia.properties(
                    &pids,
                    &policy,
                    language,
                    &mut budget,
                    &checkpoint,
                ) {
                    Ok(properties) => properties,
                    Err(error) => {
                        stop = soft_stop(&error).ok_or(error)?;
                        break 'rounds;
                    }
                };
                let (data, raw) = structured::wikidata(&capture, language, &properties)?;
                (data, properties, raw, false)
            } else {
                if !app.model.available() {
                    stop = "model_unavailable";
                    continue;
                }
                let html = capture
                    .payload
                    .get("html")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::new("UNSUPPORTED_SCHEMA", "Page HTML is missing"))?;
                // HTML is passed as bounded, explicitly untrusted semantic input. No scripts,
                // links or instructions are executed, and no completeness is attested.
                let (entities, types) = {
                    let mut store = app.store()?;
                    (
                        store.scope_entities(&caller, &scope)?,
                        store.list_types(&caller)?,
                    )
                };
                let data =
                    super::text::extract(app.model.as_ref(), &source_key, html, &entities, &types)?;
                (data, Vec::new(), Vec::new(), true)
            };
            checkpoint()?;
            let generation = {
                let store = app.store()?;
                store
                    .conn
                    .query_row(
                        "SELECT generation FROM sources WHERE source_key=?1",
                        [&source_key],
                        |r| r.get::<_, u64>(0),
                    )
                    .optional()?
                    .unwrap_or(0)
            };
            let request_key = relation::stable_id(
                "capture",
                &(
                    &source_key,
                    &capture.revision,
                    &capture.sha256,
                    "wikimedia-v1",
                    epoch,
                    &data,
                ),
            )?;
            let batch = SourceBatch {
                source_key: source_key.clone(),
                expected_generation: generation,
                scope_key: if semantic {
                    "page-partial-v1".into()
                } else {
                    "item-partial-v1".into()
                },
                complete: false,
                extractor_digest: if semantic {
                    "local-semantic-v1".into()
                } else {
                    "wikidata-restricted-v1".into()
                },
                origin_kind: if semantic {
                    "document_extraction".into()
                } else {
                    "structured_import".into()
                },
                semantic,
                data,
                capture,
                request_key,
                policy_epoch: epoch,
                job_id: Some(job_id.clone()),
                provider_properties: properties,
                raw_proposals,
            };
            let published = app.store()?.publish_source(&caller, batch)?;
            proposals += published.proposal_ids.len();
            captures.push(source_key);
            made_progress = true;
            context = app.store()?.query_goal(&caller, original_goal.clone())?;
            // Only Query Core decides whether the same typed goal is now met.
            if context.goal_coverage.goal_met {
                stop = "goal_met";
                break;
            }
            stop = "exhausted";
        }
        if context.goal_coverage.goal_met || !made_progress || budget.check().is_err() {
            break;
        }
    }
    let after: BTreeSet<_> = context
        .relations
        .iter()
        .map(|r| r.relation_id.clone())
        .collect();
    Ok(
        json!({"stop_reason":stop,"goal_coverage":context.goal_coverage,"remaining_needs":context.knowledge_needs,"added_relation_count":after.difference(&before).count(),"proposal_count":proposals,"requests":budget.requests,"response_bytes":budget.bytes,"used_providers":used,"source_keys":captures,"context_hash":context.context_hash}),
    )
}
/// Budget exhaustion or provider backoff ends an expansion but keeps the report of what it
/// already committed; any other provider error still fails the job.
fn soft_stop(error: &Error) -> Option<&'static str> {
    match error.code.as_str() {
        "BUDGET_EXHAUSTED" => Some("exhausted"),
        "RATE_LIMITED" => Some("rate_limited"),
        "DEADLINE_EXCEEDED" => Some("provider_timeout"),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_budget_and_backoff_end_an_expansion_softly() {
        let stop = |code: &str| soft_stop(&Error::new(code, "test"));
        assert_eq!(stop("BUDGET_EXHAUSTED"), Some("exhausted"));
        assert_eq!(stop("RATE_LIMITED"), Some("rate_limited"));
        assert_eq!(stop("DEADLINE_EXCEEDED"), Some("provider_timeout"));
        assert_eq!(stop("TEMPORARY_FAILURE"), None);
        assert_eq!(stop("POLICY_DENIED"), None);
    }
}
