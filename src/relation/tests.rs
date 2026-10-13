//! Native Rust qualification tests for the relation authority.
use super::condition::{Condition, Fact, Truth};
use super::query::{GoalSpec, RelationRequest};
use super::*;
use crate::acquisition::admission::*;
use crate::answer::{
    self,
    llm::{LocalLlm, ModelUse, UnavailableModel},
    AnswerStatus, AskRequest,
};
use crate::error::Result;
use crate::runtime::Profile;
use crate::storage::Store;
use serde_json::{json, Value};
use std::collections::BTreeMap;

struct Fixture {
    _directory: tempfile::TempDir,
    store: Store,
    caller: Caller,
    relation: RelationInput,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let profile = Profile::new(directory.path().join("profile")).unwrap();
        let mut store = Store::open(profile).unwrap();
        store
            .bootstrap_credentials(&"a".repeat(64), &"b".repeat(64))
            .unwrap();
        let caller = store.authenticate(&"a".repeat(64)).unwrap();
        let epoch = store.epochs().unwrap().policy;
        let mut ids = Vec::new();
        for (key, name) in [("project-a", "Project A"), ("service-b", "Service B")] {
            let entity = EntityInput {
                namespace: "demo".into(),
                kind: "concept".into(),
                canonical_key: key.into(),
                name: name.into(),
                aliases: vec![],
            };
            ids.push(
                store
                    .upsert_entity(
                        &caller,
                        EntityRequest {
                            entity,
                            scope: "local".into(),
                            expected_policy_epoch: epoch,
                        },
                    )
                    .unwrap(),
            );
        }
        let relation = RelationInput {
            relation_type: "core:DEPENDS_ON@1".into(),
            roles: BTreeMap::from([
                (
                    "dependent".into(),
                    vec![RoleValue::Entity {
                        entity_id: ids[0].clone(),
                    }],
                ),
                (
                    "dependency".into(),
                    vec![RoleValue::Entity {
                        entity_id: ids[1].clone(),
                    }],
                ),
            ]),
            qualifiers: BTreeMap::from([(
                "environment".into(),
                Literal::String("production".into()),
            )]),
            polarity: Polarity::Positive,
            modality: Modality::Asserted,
            condition: Condition::True,
            valid_time: None,
        };
        Self {
            _directory: directory,
            store,
            caller,
            relation,
        }
    }
    fn request(&self, key: &str) -> AssertRequest {
        AssertRequest {
            relation: self.relation.clone(),
            scope: "local".into(),
            idempotency_key: key.into(),
            evidence: vec![],
            epistemic_kind: EpistemicKind::Asserted,
            expected_policy_epoch: self.store.epochs().unwrap().policy,
        }
    }
    fn admit(&mut self, active: bool) {
        let policy = SourcePolicy {
            source_key: "source-json".into(),
            scope: "local".into(),
            allowed_actors: ["owner".into()].into(),
            allowed_types: ["core:DEPENDS_ON@1".into()].into(),
            entity_namespaces: ["demo".into()].into(),
            auto_admit_structured: active,
            auto_admit_semantic: false,
        };
        let epoch = self.store.epochs().unwrap().policy;
        self.store
            .admit_source(
                &self.caller,
                AdmitSourceRequest {
                    policy,
                    expected_policy_epoch: epoch,
                },
            )
            .unwrap();
    }
    fn batch(
        &self,
        revision: &str,
        generation: u64,
        complete: bool,
        data: ExtractedData,
    ) -> SourceBatch {
        SourceBatch {
            source_key: "source-json".into(),
            expected_generation: generation,
            scope_key: "facet".into(),
            complete,
            extractor_digest: "json-test-v1".into(),
            origin_kind: "structured_import".into(),
            semantic: false,
            capture: Capture::local(revision.into(), serde_json::to_value(&data).unwrap()).unwrap(),
            data,
            request_key: format!("test-{revision}"),
            policy_epoch: self.store.epochs().unwrap().policy,
            job_id: None,
            provider_properties: vec![],
            raw_proposals: vec![],
        }
    }
    fn data(&self) -> ExtractedData {
        ExtractedData {
            entities: vec![],
            assertions: vec![StructuredAssertion {
                anchor: "statement".into(),
                relation: self.relation.clone(),
                evidence: vec![],
                epistemic_kind: EpistemicKind::Asserted,
            }],
        }
    }
    fn goal(&mut self) -> GoalSpec {
        let request: AskRequest = serde_json::from_value(
            json!({"question":"What does Project A depend on in production?"}),
        )
        .unwrap();
        let entities = self.store.visible_entities(&self.caller).unwrap();
        let types = self.store.list_types(&self.caller).unwrap();
        answer::understand(&request, &entities, &types, &UnavailableModel).unwrap()
    }
}

#[test]
fn no_evidence_manual_json_withdrawal_gold() {
    let mut f = Fixture::new();
    let request = f.request("manual");
    let manual = f.store.assert_relation(&f.caller, request).unwrap();
    f.admit(true);
    let batch = f.batch("1", 0, true, f.data());
    let imported = f.store.publish_source(&f.caller, batch).unwrap();
    assert_eq!(manual.relation_id, imported.published[0].relation_id);
    let view = f
        .store
        .get_relation(
            &f.caller,
            RelationRequest {
                relation_id: manual.relation_id.clone(),
                tx_as_of: None,
            },
        )
        .unwrap();
    assert_eq!(view.assertions.len(), 2);
    assert!(view.assertions.iter().all(|a| a.evidence_refs.is_empty()));
    let epoch = f.store.epochs().unwrap().policy;
    f.store
        .withdraw_source(
            &f.caller,
            WithdrawSourceRequest {
                source_key: "source-json".into(),
                expected_generation: 1,
                expected_policy_epoch: epoch,
            },
        )
        .unwrap();
    let view = f
        .store
        .get_relation(
            &f.caller,
            RelationRequest {
                relation_id: manual.relation_id.clone(),
                tx_as_of: None,
            },
        )
        .unwrap();
    assert_eq!(view.assertions.len(), 1);
    assert_eq!(view.assertions[0].assertion_id, manual.assertion_id);
    assert!(view.assertions[0].origin.source_key.is_none());
    assert!(
        f.store
            .history(&f.caller, &manual.relation_id)
            .unwrap()
            .len()
            >= 3
    );
}
#[test]
fn partial_refresh_does_not_withdraw_but_complete_scope_can() {
    let mut f = Fixture::new();
    f.admit(true);
    let first = f.batch("1", 0, true, f.data());
    let imported = f.store.publish_source(&f.caller, first).unwrap();
    let id = imported.published[0].relation_id.clone();
    let partial = f.batch("2", 1, false, ExtractedData::default());
    assert!(f
        .store
        .publish_source(&f.caller, partial)
        .unwrap()
        .withdrawn_assertion_ids
        .is_empty());
    let view = f
        .store
        .get_relation(
            &f.caller,
            RelationRequest {
                relation_id: id.clone(),
                tx_as_of: None,
            },
        )
        .unwrap();
    assert_eq!(view.assertions.len(), 1);
    assert!(view.assertions[0].stale);
    let complete = f.batch("3", 2, true, ExtractedData::default());
    assert_eq!(
        f.store
            .publish_source(&f.caller, complete)
            .unwrap()
            .withdrawn_assertion_ids
            .len(),
        1
    );
    assert_eq!(
        f.store
            .get_relation(
                &f.caller,
                RelationRequest {
                    relation_id: id,
                    tx_as_of: None
                }
            )
            .unwrap_err()
            .code,
        "NOT_FOUND"
    );
}
#[test]
fn disabling_auto_admission_does_not_downgrade_accepted_head() {
    let mut f = Fixture::new();
    f.admit(true);
    let batch = f.batch("1", 0, true, f.data());
    let first = f.store.publish_source(&f.caller, batch).unwrap();
    f.admit(false);
    let batch = f.batch("2", 1, true, f.data());
    let result = f.store.publish_source(&f.caller, batch).unwrap();
    assert_eq!(result.proposal_ids.len(), 1);
    assert!(result.withdrawn_assertion_ids.is_empty());
    let view = f
        .store
        .get_relation(
            &f.caller,
            RelationRequest {
                relation_id: first.published[0].relation_id.clone(),
                tx_as_of: None,
            },
        )
        .unwrap();
    assert_eq!(
        view.assertions[0].publication_status,
        PublicationStatus::Active
    );
}
#[test]
fn idempotency_does_not_create_history_versions() {
    let mut f = Fixture::new();
    let request = f.request("once");
    let first = f.store.assert_relation(&f.caller, request.clone()).unwrap();
    let history = f.store.history(&f.caller, &first.relation_id).unwrap();
    let second = f.store.assert_relation(&f.caller, request).unwrap();
    assert_eq!(first.assertion_id, second.assertion_id);
    assert_eq!(
        history,
        f.store.history(&f.caller, &first.relation_id).unwrap()
    );
}
#[test]
fn persistent_inference_is_rejected_before_write() {
    let mut f = Fixture::new();
    let mut request = f.request("inferred");
    request.epistemic_kind = EpistemicKind::Inferred;
    assert_eq!(
        f.store
            .assert_relation(&f.caller, request)
            .unwrap_err()
            .code,
        "INFERENCE_DISABLED"
    );
    assert_eq!(
        f.store
            .conn
            .query_row("SELECT count(*) FROM relation_assertions", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn stale_policy_and_source_generation_are_rejected() {
    let mut f = Fixture::new();
    let request = f.request("stale");
    f.admit(true);
    assert_eq!(
        f.store
            .assert_relation(&f.caller, request)
            .unwrap_err()
            .code,
        "POLICY_CHANGED"
    );
    let batch = f.batch("1", 99, true, f.data());
    assert_eq!(
        f.store.publish_source(&f.caller, batch).unwrap_err().code,
        "VERSION_CONFLICT"
    );
    assert_eq!(
        f.store
            .conn
            .query_row("SELECT count(*) FROM source_revisions", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn empty_graph_produces_valid_unresolved_goal_without_model_or_network() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(Profile::new(directory.path().join("profile")).unwrap()).unwrap();
    store
        .bootstrap_credentials(&"a".repeat(64), &"b".repeat(64))
        .unwrap();
    let caller = store.authenticate(&"a".repeat(64)).unwrap();
    let request:AskRequest=serde_json::from_value(json!({"question":"Explain the relationships between photosynthesis, light and chlorophyll."})).unwrap();
    let goal = answer::understand(
        &request,
        &[],
        &store.list_types(&caller).unwrap(),
        &UnavailableModel,
    )
    .unwrap();
    assert_eq!(goal.seeds.len(), 3);
    assert!(goal.seeds.iter().all(|seed| seed.entity_id.is_none()));
    let context = store.query_goal(&caller, goal).unwrap();
    assert!(!context.goal_coverage.goal_met);
    assert!(context.relations.is_empty());
    assert!(matches!(
        answer::formulate(&context, &UnavailableModel)
            .unwrap()
            .status,
        AnswerStatus::NoRelationContext
    ));
    assert_eq!(
        store
            .conn
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        0
    );
}
struct AnswerFixture;
impl LocalLlm for AnswerFixture {
    fn available(&self) -> bool {
        true
    }
    fn generate(&self, purpose: ModelUse, _input: &Value) -> Result<Value> {
        assert!(matches!(purpose, ModelUse::Answer));
        Ok(
            json!({"answer":"Project A depends on Service B in production. [R1]","used_markers":["R1"]}),
        )
    }
}
#[test]
fn answer_contract_allows_evidence_free_context_but_does_not_fake_a_model() {
    let mut f = Fixture::new();
    let request = f.request("manual");
    f.store.assert_relation(&f.caller, request).unwrap();
    let goal = f.goal();
    let context = f.store.query_goal(&f.caller, goal).unwrap();
    assert!(context.goal_coverage.goal_met);
    assert!(matches!(
        answer::formulate(&context, &UnavailableModel)
            .unwrap()
            .status,
        AnswerStatus::ModelUnavailable
    ));
    let response = answer::formulate(&context, &AnswerFixture).unwrap();
    assert!(matches!(response.status, AnswerStatus::Answered));
    assert_eq!(response.relation_refs.len(), 1);
}
#[test]
fn conditions_are_three_valued_and_do_not_invent_absence() {
    let condition = Condition::Exists {
        key: "approved".into(),
    };
    assert_eq!(condition.evaluate(&BTreeMap::new()), Truth::Unknown);
    assert_eq!(
        condition.evaluate(&BTreeMap::from([(
            "approved".into(),
            Fact::AuthoritativelyAbsent
        )])),
        Truth::False
    );
    assert_eq!(
        condition.evaluate(&BTreeMap::from([(
            "approved".into(),
            Fact::Present(Literal::Boolean(false))
        )])),
        Truth::True
    );
}
#[test]
fn unknown_condition_cannot_meet_goal() {
    let mut f = Fixture::new();
    let mut request = f.request("conditional");
    request.relation.condition = Condition::Exists {
        key: "approval".into(),
    };
    f.store.assert_relation(&f.caller, request).unwrap();
    let goal = f.goal();
    let context = f.store.query_goal(&f.caller, goal).unwrap();
    assert!(!context.goal_coverage.goal_met);
    assert!(!context.unknowns.is_empty());
}
#[test]
fn one_profile_cannot_have_two_core_writers() {
    let f = Fixture::new();
    let second = Store::open(Profile::new(f._directory.path().join("profile")).unwrap());
    assert!(matches!(second,Err(error) if error.code=="CORE_UNAVAILABLE"));
}
#[test]
fn deterministic_question_without_seed_is_rejected_without_panicking() {
    let mut f = Fixture::new();
    let entities = f.store.visible_entities(&f.caller).unwrap();
    let types = f.store.list_types(&f.caller).unwrap();
    for question in ["How are related?", "how are related"] {
        let request: AskRequest = serde_json::from_value(json!({ "question": question })).unwrap();
        let error = answer::understand(&request, &entities, &types, &UnavailableModel).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT", "{question}");
    }
}
#[test]
fn deterministic_dependency_questions_keep_their_direction() {
    let mut f = Fixture::new();
    let entities = f.store.visible_entities(&f.caller).unwrap();
    let types = f.store.list_types(&f.caller).unwrap();
    for (question, seed_role, target_role) in [
        ("What does Project A depend on?", "dependent", "dependency"),
        (
            "What does Project A depend on in production?",
            "dependent",
            "dependency",
        ),
        ("What depends on Service B?", "dependency", "dependent"),
        (
            "Which services depend on Service B in production?",
            "dependency",
            "dependent",
        ),
    ] {
        let request: AskRequest = serde_json::from_value(json!({ "question": question })).unwrap();
        let goal = answer::understand(&request, &entities, &types, &UnavailableModel).unwrap();
        assert_eq!(goal.seeds.len(), 1, "{question}");
        assert_eq!(
            goal.target_slots[0].predicate,
            super::query::SuccessPredicate::FilledRole {
                seed_role: seed_role.into(),
                target_role: target_role.into(),
            },
            "{question}"
        );
    }
}
#[test]
fn explore_discloses_entities_dropped_by_the_limit() {
    let mut f = Fixture::new();
    let request: super::query::ExploreRequest =
        serde_json::from_value(json!({ "limit": 1 })).unwrap();
    let response = f.store.explore(&f.caller, request).unwrap();
    assert_eq!(response.entities.len(), 1);
    assert!(response.truncated);
}
#[test]
fn cancelling_a_job_keeps_its_committed_source_report() {
    let mut f = Fixture::new();
    let epoch = f.store.epochs().unwrap().policy;
    let job = f
        .store
        .create_job(&f.caller, "ingest_source", "local", &json!({}), epoch)
        .unwrap();
    // The running job already published one source atomically before the user cancels it.
    f.store
        .conn
        .execute(
            "UPDATE jobs SET report_json=?2 WHERE job_id=?1",
            rusqlite::params![job.job_id, r#"{"committed_sources":["src:a"]}"#],
        )
        .unwrap();
    let cancelled = f
        .store
        .cancel_job(
            &f.caller,
            crate::acquisition::expand::CancelJobRequest {
                job_id: job.job_id.clone(),
                expected_version: job.version,
            },
        )
        .unwrap();
    assert_eq!(cancelled.state, "cancelled");
    assert_eq!(cancelled.report["stop_reason"], "cancelled");
    assert_eq!(cancelled.report["committed_sources"], json!(["src:a"]));
}
#[test]
fn interrupted_first_initialization_does_not_brick_the_profile() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(directory.path().join("profile")).unwrap();
    // A crash or full disk between reserving the file and committing the schema leaves it empty.
    assert!(profile.reserve_database().unwrap());
    assert_eq!(std::fs::metadata(profile.database()).unwrap().len(), 0);
    let store = Store::open(profile.clone()).unwrap();
    store.epochs().unwrap();
    drop(store);
    // A committed database is never mistaken for an unused reservation.
    assert!(!profile.reserve_database().unwrap());
    Store::open(profile).unwrap().epochs().unwrap();
}
#[test]
fn read_connection_is_not_a_mutation_seam() {
    let f = Fixture::new();
    let reader = f.store.open_reader().unwrap();
    assert!(reader
        .execute("UPDATE system_meta SET tx_tick=999", [])
        .is_err());
}
#[test]
fn private_assertion_set_is_not_in_public_relation_version_or_history() {
    let mut f = Fixture::new();
    let request = f.request("public");
    let first = f.store.assert_relation(&f.caller, request).unwrap();
    let before = f
        .store
        .get_relation(
            &f.caller,
            RelationRequest {
                relation_id: first.relation_id.clone(),
                tx_as_of: None,
            },
        )
        .unwrap();
    let epoch = f.store.epochs().unwrap().policy;
    let principal = crate::runtime::auth::PrincipalRequest {
        actor: "private".into(),
        read_scopes: ["private".into()].into(),
        write_scopes: ["private".into()].into(),
        permissions: ["read".into(), "write".into()].into(),
        expected_policy_epoch: epoch,
    };
    f.store
        .add_principal(&f.caller, principal, &"c".repeat(64))
        .unwrap();
    let private = f.store.authenticate(&"c".repeat(64)).unwrap();
    let epoch = f.store.epochs().unwrap().policy;
    for (key, name) in [("project-a", "Project A"), ("service-b", "Service B")] {
        f.store
            .upsert_entity(
                &private,
                EntityRequest {
                    entity: EntityInput {
                        namespace: "demo".into(),
                        kind: "concept".into(),
                        canonical_key: key.into(),
                        name: name.into(),
                        aliases: vec![],
                    },
                    scope: "private".into(),
                    expected_policy_epoch: epoch,
                },
            )
            .unwrap();
    }
    let mut request = f.request("private");
    request.scope = "private".into();
    f.store.assert_relation(&private, request).unwrap();
    let after = f
        .store
        .get_relation(
            &f.caller,
            RelationRequest {
                relation_id: first.relation_id.clone(),
                tx_as_of: None,
            },
        )
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        f.store
            .history(&f.caller, &first.relation_id)
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn cancelled_operation_cannot_publish() {
    let mut f = Fixture::new();
    let request = f.request("cancelled");
    f.caller
        .cancelled
        .store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(
        f.store
            .assert_relation(&f.caller, request)
            .unwrap_err()
            .code,
        "CANCELLED"
    );
}

#[test]
fn opposite_polarity_is_disclosed_as_a_conflict_and_blocks_goal_coverage() {
    let mut f = Fixture::new();
    f.store
        .assert_relation(&f.caller, f.request("positive"))
        .unwrap();
    let mut request = f.request("negative");
    request.relation.polarity = Polarity::Negative;
    f.store.assert_relation(&f.caller, request).unwrap();
    let goal = f.goal();
    let context = f.store.query_goal(&f.caller, goal).unwrap();
    assert_eq!(context.conflicts.len(), 1);
    assert_eq!(context.conflicts[0].state, "potential_conflict");
    assert!(!context.goal_coverage.goal_met);
    assert!(context
        .unknowns
        .iter()
        .any(|reason| reason.contains("negative")));
}

#[test]
fn valid_time_outside_the_requested_instant_is_not_coverage() {
    let mut f = Fixture::new();
    let mut request = f.request("timed");
    request.relation.valid_time = Some(ValidTime {
        start: Some(1_000),
        end: Some(2_000),
    });
    f.store.assert_relation(&f.caller, request).unwrap();
    let mut goal = f.goal();
    assert!(
        f.store
            .query_goal(&f.caller, goal.clone())
            .unwrap()
            .goal_coverage
            .goal_met
    );
    goal.valid_at = Some(5_000);
    let context = f.store.query_goal(&f.caller, goal.clone()).unwrap();
    assert!(!context.goal_coverage.goal_met);
    assert!(context
        .unknowns
        .iter()
        .any(|reason| reason.contains("domain time")));
    goal.valid_at = Some(1_500);
    assert!(
        f.store
            .query_goal(&f.caller, goal)
            .unwrap()
            .goal_coverage
            .goal_met
    );
}

#[test]
fn nary_literal_roles_are_published_and_read_back_as_typed_bindings() {
    let mut f = Fixture::new();
    let subject = f.relation.roles["dependent"][0].clone();
    let mut request = f.request("property");
    request.relation = RelationInput {
        relation_type: "core:HAS_PROPERTY@1".into(),
        roles: BTreeMap::from([
            ("subject".into(), vec![subject]),
            (
                "property".into(),
                vec![RoleValue::Literal {
                    literal: Literal::String("headcount".into()),
                }],
            ),
            (
                "value".into(),
                vec![RoleValue::Literal {
                    literal: Literal::Integer(12),
                }],
            ),
        ]),
        qualifiers: BTreeMap::new(),
        polarity: Polarity::Positive,
        modality: Modality::Asserted,
        condition: Condition::True,
        valid_time: None,
    };
    let published = f.store.assert_relation(&f.caller, request).unwrap();
    let view = f
        .store
        .get_relation(
            &f.caller,
            RelationRequest {
                relation_id: published.relation_id.clone(),
                tx_as_of: None,
            },
        )
        .unwrap();
    assert_eq!(view.relation.roles.len(), 3);
    assert_eq!(
        view.relation.roles["value"],
        vec![RoleValue::Literal {
            literal: Literal::Integer(12)
        }]
    );
    let bindings: u64 = f
        .store
        .conn
        .query_row(
            "SELECT count(*) FROM relation_role_bindings WHERE relation_id=?1",
            [&published.relation_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bindings, 3);
    let mut wrong = f.request("wrong-type");
    wrong.relation = view.relation.clone();
    wrong.relation.roles.insert(
        "value".into(),
        vec![RoleValue::Literal {
            literal: Literal::Decimal("1.50".into()),
        }],
    );
    assert_eq!(
        f.store.assert_relation(&f.caller, wrong).unwrap_err().code,
        "INVALID_INPUT"
    );
}

#[test]
fn auto_expansion_without_independent_consent_creates_no_job_and_no_egress() {
    use crate::acquisition::providers::{OfflineRequest, ProviderPolicy, ProviderPolicyRequest};
    let Fixture {
        _directory,
        mut store,
        caller,
        ..
    } = Fixture::new();
    let epoch = store.epochs().unwrap().policy;
    store
        .set_offline(
            &caller,
            OfflineRequest {
                offline: false,
                expected_policy_epoch: epoch,
            },
        )
        .unwrap();
    // Consent exists for the term, but fetching stays independently disabled.
    let epoch = store.epochs().unwrap().policy;
    store
        .set_provider_policy(
            &caller,
            ProviderPolicyRequest {
                policy: ProviderPolicy {
                    provider_id: "wikidata".into(),
                    scope: "local".into(),
                    allowed_actors: ["owner".into()].into(),
                    languages: ["en".into()].into(),
                    approved_public_terms: ["Photosynthesis".into()].into(),
                    may_fetch: false,
                    auto_register_provider_types: false,
                    auto_admit_structured: false,
                    auto_admit_semantic: false,
                    semantic_types: Default::default(),
                    user_agent: format!("Huda/{} (test@example.invalid)", crate::VERSION),
                    ttl_seconds: 3600,
                },
                expected_policy_epoch: epoch,
            },
        )
        .unwrap();
    let app = crate::api::App::new(store, std::sync::Arc::new(UnavailableModel));
    let request: AskRequest = serde_json::from_value(
        json!({"question":"What does Photosynthesis depend on?","expansion_mode":"auto"}),
    )
    .unwrap();
    let response = app.ask(&caller, request.clone()).unwrap();
    assert!(response.expansion.is_none());
    assert!(!response.goal_coverage.goal_met);
    assert!(!response.knowledge_needs.is_empty());
    assert!(response
        .degraded_components
        .iter()
        .any(|d| d.starts_with("knowledge_expansion_blocked")));
    // An explicit expansion request is refused before any HTTP request as well.
    let error = app
        .dispatch(
            &caller,
            "expand_knowledge",
            serde_json::to_value(&request).unwrap(),
        )
        .unwrap_err();
    assert_eq!(error.code, "POLICY_DENIED");
    let jobs: u64 = app
        .store()
        .unwrap()
        .conn
        .query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(jobs, 0);
    assert_eq!(app.running_jobs(), 0);
}
