//! Common application use cases. All transports are adapters, never additional writers.
pub mod contracts;
#[cfg(feature = "rest")]
pub mod rest;

use crate::acquisition::{
    admission::*,
    expand::{self, CancelJobRequest, JobRequest, JobView},
};
use crate::answer::{self, llm::LocalLlm, AskRequest, AskResponse, ExpansionMode};
use crate::error::{require, Error, Result};
use crate::relation::{query::*, *};
use crate::runtime::auth::OsSecrets;
use crate::storage::Store;
use rusqlite::{params, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex, MutexGuard,
};

#[derive(Clone)]
pub struct App {
    storage: Arc<Mutex<Store>>,
    pub(crate) model: Arc<dyn LocalLlm>,
    pub(crate) wikimedia: Arc<crate::acquisition::providers::wikimedia::Wikimedia>,
    active_jobs: Arc<AtomicUsize>,
    activity: Arc<AtomicU64>,
    cancellations: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Empty {}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryRequest {
    pub relation_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BackupRequest {
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRequest {
    pub source_key: String,
}
struct JobSlot(Arc<AtomicUsize>);
impl Drop for JobSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl App {
    pub fn new(store: Store, model: Arc<dyn LocalLlm>) -> Self {
        Self {
            storage: Arc::new(Mutex::new(store)),
            model,
            wikimedia: Arc::new(Default::default()),
            active_jobs: Arc::new(AtomicUsize::new(0)),
            activity: Arc::new(AtomicU64::new(now_ms())),
            cancellations: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
    pub(crate) fn store(&self) -> Result<MutexGuard<'_, Store>> {
        self.storage.lock().map_err(|_| {
            Error::new(
                "READ_ONLY_RECOVERY",
                "Core state is poisoned; restart and inspect without wiping data",
            )
        })
    }
    pub fn authenticate(&self, token: &str, deadline_ms: u64) -> Result<Caller> {
        require(
            // One second of tolerance absorbs clock steps between the client's and the
            // Core's SystemTime reads; it does not extend the 120-second request bound.
            deadline_ms >= now_ms() && deadline_ms <= now_ms() + 121_000,
            "INVALID_INPUT",
            "Request deadline is outside allowed bounds",
        )?;
        let mut caller = self.store()?.authenticate(token)?;
        caller.deadline_ms = deadline_ms;
        Ok(caller)
    }
    pub fn last_activity(&self) -> u64 {
        self.activity.load(Ordering::Acquire)
    }
    pub fn running_jobs(&self) -> usize {
        self.active_jobs.load(Ordering::Acquire)
    }
    pub fn dispatch(&self, caller: &Caller, method: &str, input: Value) -> Result<Value> {
        caller.check()?;
        self.activity.store(now_ms(), Ordering::Release);
        // Deserializing a transport payload never creates a Caller or grants permissions.
        match method {
            "status" => {
                let _: Empty = decode(input)?;
                self.status(caller)
            }
            "upsert_entity" => {
                encode(json!({"entity_id":self.store()?.upsert_entity(caller,decode(input)?)?}))
            }
            "assert_relation" => encode(self.store()?.assert_relation(caller, decode(input)?)?),
            "withdraw_relation_assertion" => {
                encode(self.store()?.withdraw_assertion(caller, decode(input)?)?)
            }
            "review_assertion" => encode(self.store()?.review_assertion(caller, decode(input)?)?),
            "register_relation_type" => {
                encode(self.store()?.register_type(caller, decode(input)?)?)
            }
            "list_relation_types" => {
                let _: Empty = decode(input)?;
                encode(self.store()?.list_types(caller)?)
            }
            "search_knowledge" | "explore_relations" => {
                encode(self.store()?.explore(caller, decode(input)?)?)
            }
            "get_relation" => encode(self.store()?.get_relation(caller, decode(input)?)?),
            "get_relation_history" => {
                let request: HistoryRequest = decode(input)?;
                encode(self.store()?.history(caller, &request.relation_id)?)
            }
            "ask_huda" => encode(self.ask(caller, decode(input)?)?),
            "expand_knowledge" => {
                let request: AskRequest = decode(input)?;
                let (context, epoch) = self.context(caller, &request)?;
                encode(self.start_expansion(caller, context, epoch)?)
            }
            "get_knowledge_providers" => {
                let _: Empty = decode(input)?;
                self.store()?.provider_status(caller)
            }
            "set_provider_policy" => {
                self.store()?.set_provider_policy(caller, decode(input)?)?;
                encode(self.store()?.epochs()?)
            }
            "set_offline" => {
                self.store()?.set_offline(caller, decode(input)?)?;
                encode(self.store()?.epochs()?)
            }
            "admit_source" => encode(self.store()?.admit_source(caller, decode(input)?)?),
            "get_source" => {
                let request: SourceRequest = decode(input)?;
                self.source(caller, &request.source_key)
            }
            "withdraw_source" => encode(self.store()?.withdraw_source(caller, decode(input)?)?),
            "ingest_source" => encode(self.ingest(caller, decode(input)?)?),
            "get_job" => {
                let request: JobRequest = decode(input)?;
                encode(self.store()?.get_job(caller, &request.job_id)?)
            }
            "cancel_job" => {
                let request: CancelJobRequest = decode(input)?;
                let id = request.job_id.clone();
                let response = self.store()?.cancel_job(caller, request)?;
                if let Some(flag) = self
                    .cancellations
                    .lock()
                    .map_err(|_| Error::new("INTERNAL_ERROR", "Cancellation state is unavailable"))?
                    .get(&id)
                {
                    flag.store(true, Ordering::Release);
                }
                encode(response)
            }
            "list_proposals" => {
                let _: Empty = decode(input)?;
                self.proposals(caller)
            }
            "backup" => {
                let request: BackupRequest = decode(input)?;
                encode(json!({"backup":self.store()?.backup(caller,&request.name)?}))
            }
            "add_principal" => {
                let request: crate::runtime::auth::PrincipalRequest = decode(input)?;
                caller.require("admin")?;
                let profile = self.store()?.profile.clone();
                let token = OsSecrets::create(&profile, &request.actor)?;
                self.store()?.add_principal(caller, request, &token)?;
                Ok(json!({"created":true,"credential_location":"os_secret_store"}))
            }
            "revoke_principal" => {
                self.store()?.revoke_principal(caller, decode(input)?)?;
                encode(self.store()?.epochs()?)
            }
            "configure_bridge" => {
                self.store()?.configure_bridge(caller, decode(input)?)?;
                encode(self.store()?.epochs()?)
            }
            "bridge_status" => {
                let _: Empty = decode(input)?;
                self.store()?.bridge_status(caller)
            }
            "capture_bridge" => encode(self.capture_bridge(caller, decode(input)?)?),
            "openapi" => {
                caller.require("read")?;
                let _: Empty = decode(input)?;
                Ok(contracts::openapi())
            }
            _ => Err(Error::new(
                "METHOD_NOT_FOUND",
                "Application operation is not supported",
            )),
        }
    }
    fn status(&self, caller: &Caller) -> Result<Value> {
        caller.require("read")?;
        let store = self.store()?;
        let visible:u64=store.conn.query_row("SELECT count(DISTINCT relation_id) FROM relation_assertions WHERE current_status='active' AND scope IN(SELECT value FROM json_each(?1))",[serde_json::to_string(&caller.read_scopes)?],|r|r.get(0))?;
        Ok(
            json!({"product":crate::APP_NAME,"identifier":crate::APP_ID,"version":crate::VERSION,"schema_version":crate::storage::SCHEMA_VERSION,"ipc_version":crate::IPC_VERSION,"mcp_protocol":crate::MCP_VERSION,"epochs":store.epochs()?,"health":if self.model.available(){"READY_CORE"}else{"GRAPH_ONLY"},"local_model_configured":self.model.available(),"visible_active_relations":visible,"persistent_inference":false,"release_qualification":"UNQUALIFIED_SOURCE_CANDIDATE"}),
        )
    }
    fn context(&self, caller: &Caller, request: &AskRequest) -> Result<(RelationContext, u64)> {
        caller.require("read")?;
        let (entities, types, epoch) = {
            let mut store = self.store()?;
            (
                store.visible_entities(caller)?,
                store.list_types(caller)?,
                store.policy_epoch()?,
            )
        };
        // No DB lock or snapshot is held during the local model call.
        let goal = answer::understand(request, &entities, &types, self.model.as_ref())?;
        let mut store = self.store()?;
        store.check_policy(caller, epoch)?;
        Ok((store.query_goal(caller, goal)?, epoch))
    }
    pub fn ask(&self, caller: &Caller, request: AskRequest) -> Result<AskResponse> {
        let (context, epoch) = self.context(caller, &request)?;
        let mut response = answer::formulate(&context, self.model.as_ref())?;
        self.store()?.check_policy(caller, epoch)?;
        if request.expansion_mode == ExpansionMode::Auto
            && !context.goal_coverage.goal_met
            && caller.can("expand")
        {
            if !expand::eligible(self, caller, &context, epoch)?.is_empty() {
                match self.start_expansion(caller, context, epoch) {
                    Ok(job) => response.expansion = Some(job),
                    Err(error) if error.code == "BACKPRESSURE" => response
                        .degraded_components
                        .push("expansion_backpressure".into()),
                    Err(error) => return Err(error),
                }
            } else {
                response
                    .degraded_components
                    .push("knowledge_expansion_blocked_or_no_approved_public_terms".into());
            }
        }
        Ok(response)
    }
    fn start_expansion(
        &self,
        caller: &Caller,
        context: RelationContext,
        epoch: u64,
    ) -> Result<JobView> {
        require(
            !context.goal_coverage.goal_met,
            "GOAL_ALREADY_MET",
            "Required goal slots are already covered",
        )?;
        let eligible = expand::eligible(self, caller, &context, epoch)?;
        let scope = eligible.first().map(|r| r.2.clone()).ok_or_else(|| {
            Error::new(
                "POLICY_DENIED",
                "No authorized unmet need can be fetched; zero HTTP requests were made",
            )
        })?;
        let goal = serde_json::to_value(&context.goal)?;
        self.spawn_job(
            caller,
            "knowledge_expansion",
            &scope,
            &goal,
            epoch,
            move |app, caller, id| expand::run(app, caller, id, context, epoch),
        )
    }
    fn ingest(&self, caller: &Caller, request: IngestRequest) -> Result<JobView> {
        caller.require("ingest")?;
        request.validate()?;
        let scope = {
            let store = self.store()?;
            store.check_policy(caller, request.expected_policy_epoch)?;
            store.source_generation(caller, &request.source_key)?;
            store.conn.query_row(
                "SELECT scope FROM sources WHERE source_key=?1",
                [&request.source_key],
                |r| r.get::<_, String>(0),
            )?
        };
        let goal = json!({"source_key":request.source_key,"revision":request.revision,"scope_key":request.extraction_scope_key});
        let epoch = request.expected_policy_epoch;
        self.spawn_job(
            caller,
            "source_ingestion",
            &scope,
            &goal,
            epoch,
            move |app, caller, id| {
                let capture = Capture::local(
                    request.revision.clone(),
                    serde_json::to_value(&request.payload)?,
                )?;
                let semantic = matches!(
                    &request.payload,
                    SourcePayload::Text { .. } | SourcePayload::Markdown { .. }
                );
                let (data, extractor, origin) = match &request.payload {
                    SourcePayload::Relations { data } => {
                        (data.clone(), "structured-json-v1", "structured_import")
                    }
                    SourcePayload::Text { text } | SourcePayload::Markdown { text } => {
                        let (entities, types) = {
                            let mut store = app.store()?;
                            (store.visible_entities(&caller)?, store.list_types(&caller)?)
                        };
                        (
                            crate::acquisition::text::extract(
                                app.model.as_ref(),
                                &request.source_key,
                                text,
                                &entities,
                                &types,
                            )?,
                            "local-semantic-v1",
                            "document_extraction",
                        )
                    }
                    SourcePayload::Code {
                        text,
                        language,
                        relative_path,
                        root_namespace,
                    } => {
                        #[cfg(feature = "code-adapter")]
                        {
                            (
                                crate::acquisition::code::extract(
                                    text,
                                    language,
                                    relative_path,
                                    root_namespace,
                                )?,
                                "tree-sitter-syntax-v1",
                                "specialized_adapter",
                            )
                        }
                        #[cfg(not(feature = "code-adapter"))]
                        {
                            let _ = (text, language, relative_path, root_namespace);
                            return Err(Error::new(
                                "ADAPTER_UNAVAILABLE",
                                "Build with code-adapter to ingest explicit code inputs",
                            ));
                        }
                    }
                };
                let batch = SourceBatch {
                    source_key: request.source_key,
                    expected_generation: request.expected_generation,
                    scope_key: request.extraction_scope_key,
                    complete: request.coverage_complete_for_scope,
                    extractor_digest: extractor.into(),
                    origin_kind: origin.into(),
                    semantic,
                    data,
                    capture,
                    request_key: format!("ingest:{}", request.idempotency_key),
                    policy_epoch: epoch,
                    job_id: Some(id),
                    provider_properties: Vec::new(),
                    raw_proposals: Vec::new(),
                };
                let imported = app.store()?.publish_source(&caller, batch)?;
                Ok(json!({"stop_reason":"ingested","import":imported}))
            },
        )
    }
    pub(crate) fn spawn_job<F>(
        &self,
        caller: &Caller,
        kind: &str,
        scope: &str,
        goal: &Value,
        epoch: u64,
        work: F,
    ) -> Result<JobView>
    where
        F: FnOnce(App, Caller, String) -> Result<Value> + Send + 'static,
    {
        self.active_jobs
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 4).then_some(n + 1)
            })
            .map_err(|_| Error::new("BACKPRESSURE", "Core job capacity is full"))?;
        let slot = JobSlot(self.active_jobs.clone());
        let job = self.store()?.create_job(caller, kind, scope, goal, epoch)?;
        let mut job_caller = caller.clone();
        job_caller.deadline_ms = now_ms() + 120_000;
        job_caller.cancelled = Arc::new(AtomicBool::new(false));
        self.cancellations
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "Cancellation state unavailable"))?
            .insert(job.job_id.clone(), job_caller.cancelled.clone());
        let app = self.clone();
        let id = job.job_id.clone();
        let failed_caller = job_caller.clone();
        let spawned = std::thread::Builder::new()
            .name("huda-acquisition".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let _slot = slot;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    app.store()?.begin_job(&job_caller, &id, epoch)?;
                    work(app.clone(), job_caller.clone(), id.clone())
                }))
                .unwrap_or_else(|_| {
                    Err(Error::new(
                        "WORKER_FAILED",
                        "Acquisition worker failed; no repair or source deletion was attempted",
                    ))
                });
                if let Ok(mut store) = app.store() {
                    let _ = store.finish_job(&job_caller, &id, &result);
                }
                if let Ok(mut flags) = app.cancellations.lock() {
                    flags.remove(&id);
                }
            });
        if spawned.is_err() {
            let failure = Err(Error::new(
                "BACKPRESSURE",
                "Could not start the acquisition worker",
            ));
            self.store()?
                .finish_job(&failed_caller, &job.job_id, &failure)?;
            if let Ok(mut flags) = self.cancellations.lock() {
                flags.remove(&job.job_id);
            }
            return Err(Error::new(
                "BACKPRESSURE",
                "Could not start the acquisition worker",
            ));
        }
        Ok(job)
    }
    fn source(&self, caller: &Caller, key: &str) -> Result<Value> {
        caller.require("ingest")?;
        let store = self.store()?;
        let row:Option<(String,String,u64,Option<String>,bool)>=store.conn.query_row("SELECT scope,policy_json,generation,current_revision,withdrawn FROM sources WHERE source_key=?1 AND owner=?2",params![key,caller.actor],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let (scope, policy, generation, revision, withdrawn) = row.ok_or_else(Error::not_found)?;
        require(caller.read(&scope), "NOT_FOUND", "Source is not visible")?;
        Ok(
            json!({"source_key":key,"scope":scope,"policy":serde_json::from_str::<Value>(&policy)?,"generation":generation,"revision":revision,"withdrawn":withdrawn}),
        )
    }
    fn proposals(&self, caller: &Caller) -> Result<Value> {
        caller.require("read")?;
        let store = self.store()?;
        let mut statement=store.conn.prepare("SELECT proposal_id,reason,payload_json FROM relation_proposals WHERE scope IN(SELECT value FROM json_each(?1)) AND actor=?2 ORDER BY proposal_id LIMIT 101")?;
        let rows = statement
            .query_map(
                params![serde_json::to_string(&caller.read_scopes)?, caller.actor],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let truncated = rows.len() > 100;
        let mut values = Vec::new();
        for (id, reason, payload) in rows.into_iter().take(100) {
            values.push(json!({"proposal_id":id,"reason":reason,"payload":serde_json::from_str::<Value>(&payload)?}));
        }
        Ok(json!({"proposals":values,"truncated":truncated}))
    }
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(Into::into)
}
fn encode<T: Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(Into::into)
}
