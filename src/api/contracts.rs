//! Schemas are derived from the exact handler DTOs, not independently handwritten contracts.
use super::{BackupRequest, Empty, HistoryRequest, SourceRequest};
use crate::acquisition::{admission::*, expand::*, providers::*};
use crate::answer::AskRequest;
use crate::relation::{query::*, *};
use serde_json::{json, Map, Value};

pub struct Operation {
    pub name: &'static str,
    pub verb: &'static str,
    pub path: &'static str,
    pub tool: Option<&'static str>,
    pub description: &'static str,
    pub read_only: bool,
}
pub const OPERATIONS: &[Operation] = &[
    Operation {
        name: "status",
        verb: "get",
        path: "/api/v1/status",
        tool: Some("huda_status"),
        description: "Read caller-visible Huda health and version information.",
        read_only: true,
    },
    Operation {
        name: "upsert_entity",
        verb: "post",
        path: "/api/v1/entities",
        tool: Some("upsert_entity"),
        description: "Create a scoped canonical entity or add its scoped labels.",
        read_only: false,
    },
    Operation {
        name: "assert_relation",
        verb: "post",
        path: "/api/v1/relations",
        tool: Some("assert_relation"),
        description:
            "Assert a typed relation. Evidence is optional; origin and policy checks are mandatory.",
        read_only: false,
    },
    Operation {
        name: "get_relation",
        verb: "get",
        path: "/api/v1/relations/{relation_id}",
        tool: None,
        description: "Read a relation only through visible assertions.",
        read_only: true,
    },
    Operation {
        name: "get_relation_history",
        verb: "get",
        path: "/api/v1/relations/{relation_id}/history",
        tool: None,
        description: "Read immutable caller-visible assertion history.",
        read_only: true,
    },
    Operation {
        name: "withdraw_relation_assertion",
        verb: "delete",
        path: "/api/v1/assertions/{assertion_id}",
        tool: Some("withdraw_relation_assertion"),
        description: "Withdraw one owned direct assertion with version and policy CAS.",
        read_only: false,
    },
    Operation {
        name: "review_assertion",
        verb: "post",
        path: "/api/v1/assertions/review",
        tool: None,
        description: "Explicit privileged review of an owned proposed assertion.",
        read_only: false,
    },
    Operation {
        name: "list_relation_types",
        verb: "get",
        path: "/api/v1/relation-types",
        tool: Some("list_relation_types"),
        description: "Read registered type versions, roles and qualifier signatures.",
        read_only: true,
    },
    Operation {
        name: "register_relation_type",
        verb: "post",
        path: "/api/v1/relation-types",
        tool: None,
        description: "Privileged deterministic registry append; never an open MCP tool.",
        read_only: false,
    },
    Operation {
        name: "search_knowledge",
        verb: "post",
        path: "/api/v1/search",
        tool: Some("search_knowledge"),
        description: "Bounded, assertion-ACL-filtered entity and relation search.",
        read_only: true,
    },
    Operation {
        name: "explore_relations",
        verb: "post",
        path: "/api/v1/relations/explore",
        tool: Some("explore_relations"),
        description: "Explore typed role bindings, origin, history and optional evidence.",
        read_only: true,
    },
    Operation {
        name: "ask_huda",
        verb: "post",
        path: "/api/v1/answers",
        tool: Some("ask_huda"),
        description:
            "Answer from authorized Relation Context. Auto expansion never grants consent.",
        read_only: false,
    },
    Operation {
        name: "expand_knowledge",
        verb: "post",
        path: "/api/v1/knowledge/expand",
        tool: Some("expand_knowledge"),
        description:
            "Start one bounded acquisition job for internally derived goal-specific needs.",
        read_only: false,
    },
    Operation {
        name: "get_knowledge_providers",
        verb: "get",
        path: "/api/v1/knowledge/providers",
        tool: Some("get_knowledge_providers"),
        description: "Read approved capabilities and separate fetch/admission/registration status.",
        read_only: true,
    },
    Operation {
        name: "set_provider_policy",
        verb: "post",
        path: "/api/v1/knowledge/provider-policy",
        tool: None,
        description: "Privileged provider consent update with policy CAS.",
        read_only: false,
    },
    Operation {
        name: "set_offline",
        verb: "post",
        path: "/api/v1/settings/offline",
        tool: None,
        description: "Privileged offline-policy change, invalidating stale jobs.",
        read_only: false,
    },
    Operation {
        name: "admit_source",
        verb: "post",
        path: "/api/v1/source-admissions",
        tool: None,
        description: "Admit or update an owned source policy and namespace allowlist.",
        read_only: false,
    },
    Operation {
        name: "ingest_source",
        verb: "post",
        path: "/api/v1/sources",
        tool: Some("ingest_source"),
        description: "Start real, bounded source ingestion through the sole publication boundary.",
        read_only: false,
    },
    Operation {
        name: "get_source",
        verb: "get",
        path: "/api/v1/sources/{source_key}",
        tool: None,
        description: "Read source generation and admission state.",
        read_only: true,
    },
    Operation {
        name: "withdraw_source",
        verb: "delete",
        path: "/api/v1/sources/{source_key}",
        tool: None,
        description:
            "Withdraw only this owned source's assertions; retain other origins and history.",
        read_only: false,
    },
    Operation {
        name: "get_job",
        verb: "get",
        path: "/api/v1/jobs/{job_id}",
        tool: Some("get_ingestion_status"),
        description: "Read actual authorized job state and committed source deltas.",
        read_only: true,
    },
    Operation {
        name: "cancel_job",
        verb: "post",
        path: "/api/v1/jobs/cancel",
        tool: Some("cancel_job"),
        description:
            "Cancel an owned job with version CAS; already committed transactions are retained.",
        read_only: false,
    },
    Operation {
        name: "list_proposals",
        verb: "get",
        path: "/api/v1/proposals",
        tool: None,
        description: "Read bounded caller-visible relation/type proposals without publishing them.",
        read_only: true,
    },
    Operation {
        name: "backup",
        verb: "post",
        path: "/api/v1/backups",
        tool: None,
        description: "Privileged non-overwriting SQLite snapshot backup.",
        read_only: false,
    },
    Operation {
        name: "add_principal",
        verb: "post",
        path: "/api/v1/principals",
        tool: None,
        description: "Privileged scoped capability creation in the OS secret store.",
        read_only: false,
    },
    Operation {
        name: "revoke_principal",
        verb: "post",
        path: "/api/v1/principals/revoke",
        tool: None,
        description: "Privileged capability revocation and policy epoch update.",
        read_only: false,
    },
    Operation {
        name: "configure_bridge",
        verb: "post",
        path: "/api/v1/bridge/sources",
        tool: None,
        description: "Approve an exact sandboxed read-only MCP Bridge configuration.",
        read_only: false,
    },
    Operation {
        name: "bridge_status",
        verb: "get",
        path: "/api/v1/bridge/status",
        tool: None,
        description: "Read authorized Bridge health without credentials or raw arguments.",
        read_only: true,
    },
    Operation {
        name: "capture_bridge",
        verb: "post",
        path: "/api/v1/bridge/captures",
        tool: None,
        description: "Start an approved bounded MCP read capture through normal admission.",
        read_only: false,
    },
];
macro_rules! schema {
    ($t:ty) => {
        serde_json::to_value(schemars::schema_for!($t)).expect("Derived schemas serialize as JSON")
    };
}
pub fn input_schema(operation: &str) -> Value {
    match operation {
        "upsert_entity" => schema!(EntityRequest),
        "assert_relation" => schema!(AssertRequest),
        "withdraw_relation_assertion" => schema!(WithdrawRequest),
        "review_assertion" => schema!(crate::relation::publish::ReviewAssertionRequest),
        "register_relation_type" => schema!(RegisterTypeRequest),
        "get_relation" => schema!(RelationRequest),
        "get_relation_history" => schema!(HistoryRequest),
        "search_knowledge" | "explore_relations" => schema!(ExploreRequest),
        "ask_huda" | "expand_knowledge" => schema!(AskRequest),
        "set_provider_policy" => schema!(ProviderPolicyRequest),
        "set_offline" => schema!(OfflineRequest),
        "admit_source" => schema!(AdmitSourceRequest),
        "ingest_source" => schema!(IngestRequest),
        "get_source" => schema!(SourceRequest),
        "withdraw_source" => schema!(WithdrawSourceRequest),
        "get_job" => schema!(JobRequest),
        "cancel_job" => schema!(CancelJobRequest),
        "backup" => schema!(BackupRequest),
        "add_principal" => schema!(crate::runtime::auth::PrincipalRequest),
        "revoke_principal" => schema!(crate::runtime::auth::RevokeRequest),
        "configure_bridge" => schema!(crate::bridge::ConfigureBridgeRequest),
        "capture_bridge" => schema!(crate::bridge::CaptureBridgeRequest),
        _ => schema!(Empty),
    }
}
fn output_schema(operation: &str) -> Value {
    match operation {
        "assert_relation" | "withdraw_relation_assertion" | "review_assertion" => {
            schema!(Published)
        }
        "get_relation" => schema!(RelationView),
        "get_relation_history" => schema!(Vec<RelationView>),
        "list_relation_types" => schema!(Vec<RelationType>),
        "register_relation_type" => schema!(RelationType),
        "ask_huda" => schema!(crate::answer::AskResponse),
        "get_job" | "cancel_job" | "ingest_source" | "expand_knowledge" | "capture_bridge" => {
            schema!(JobView)
        }
        "explore_relations" | "search_knowledge" => schema!(ExploreResponse),
        "admit_source" => schema!(SourcePolicy),
        _ => {
            json!({"description":"Operational result; fields are produced by the common handler."})
        }
    }
}
pub fn tools() -> Vec<Value> {
    let mut tools:Vec<_>=OPERATIONS.iter().filter_map(|op|op.tool.map(|name|json!({"name":name,"description":op.description,"inputSchema":input_schema(op.name),"outputSchema":output_schema(op.name),"annotations":{"readOnlyHint":op.read_only,"destructiveHint":op.name.starts_with("withdraw")||op.name=="cancel_job","openWorldHint":matches!(op.name,"ask_huda"|"expand_knowledge")}}))).collect();
    tools.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    tools
}
pub fn tool_operation(name: &str) -> Option<&'static str> {
    OPERATIONS
        .iter()
        .find(|op| op.tool == Some(name))
        .map(|op| op.name)
}
pub fn openapi() -> Value {
    let mut components = Map::new();
    let mut paths = Map::new();
    for op in OPERATIONS {
        let input_name = format!("{}Input", op.name);
        let mut input = input_schema(op.name);
        hoist(&mut input, &mut components);
        components.insert(input_name.clone(), input.clone());
        let output_name = format!("{}Output", op.name);
        let mut output = output_schema(op.name);
        hoist(&mut output, &mut components);
        components.insert(output_name.clone(), output);
        let accepted = matches!(
            op.name,
            "ingest_source" | "expand_knowledge" | "capture_bridge"
        );
        let mut operation = json!({"operationId":op.name,"description":op.description,"security":[{"localCapability":[]}],"responses":{(if accepted{"202"}else{"200"}):{"description":if accepted{"A real bounded job was accepted."}else{"Successful authorized operation."},"content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{output_name}")}}}},"default":{"description":"Explicit redacted error, never a fabricated successful job.","content":{"application/json":{"schema":{"type":"object","required":["code","message","correlation_id"],"properties":{"code":{"type":"string"},"message":{"type":"string"},"correlation_id":{"type":"string"}}}}}}}});
        let path_parameter = op.path.split('{').nth(1).and_then(|p| p.split('}').next());
        if op.verb == "get" || op.verb == "delete" {
            let mut parameters = Vec::new();
            if let Some(properties) = input.get("properties").and_then(Value::as_object) {
                for (name, schema) in properties {
                    let is_path = path_parameter == Some(name.as_str());
                    let required = is_path
                        || input
                            .get("required")
                            .and_then(Value::as_array)
                            .is_some_and(|v| v.iter().any(|x| x == name));
                    parameters.push(json!({"name":name,"in":if is_path{"path"}else{"query"},"required":required,"schema":schema}));
                }
            }
            operation["parameters"] = parameters.into();
        } else {
            operation["requestBody"] = json!({"required":true,"content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{input_name}")}}}});
        }
        paths.entry(op.path).or_insert_with(|| json!({}))[op.verb] = operation;
    }
    paths.insert("/openapi.json".into(),json!({"get":{"operationId":"openapi","security":[{"localCapability":[]}],"responses":{"200":{"description":"This generated OpenAPI 3.1.2 contract."}}}}));
    json!({"openapi":"3.1.2","info":{"title":"Huda local application API","version":crate::VERSION,"description":"Unqualified source candidate. Core permission checks are mandatory; localhost is not trusted."},"jsonSchemaDialect":"https://json-schema.org/draft/2020-12/schema","servers":[{"url":"http://127.0.0.1:{port}","variables":{"port":{"default":"7377"}}}],"paths":paths,"components":{"schemas":components,"securitySchemes":{"localCapability":{"type":"http","scheme":"bearer","description":"Per-client scoped capability from the OS secret store."}}}})
}
fn hoist(schema: &mut Value, components: &mut Map<String, Value>) {
    if let Some(definitions) = schema
        .as_object_mut()
        .and_then(|map| map.remove("$defs"))
        .and_then(|v| v.as_object().cloned())
    {
        for (name, mut definition) in definitions {
            rewrite_refs(&mut definition);
            components.entry(name).or_insert(definition);
        }
    }
    rewrite_refs(schema);
}
fn rewrite_refs(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get_mut("$ref") {
                if let Some(tail) = reference.strip_prefix("#/$defs/") {
                    *reference = format!("#/components/schemas/{tail}");
                }
            }
            for child in map.values_mut() {
                rewrite_refs(child);
            }
        }
        Value::Array(array) => {
            for child in array {
                rewrite_refs(child);
            }
        }
        _ => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_has_no_privileged_registry_or_erasure_tool() {
        for forbidden in [
            "register_relation_type",
            "set_provider_policy",
            "withdraw_source",
            "add_principal",
        ] {
            assert!(tool_operation(forbidden).is_none());
        }
    }
    #[test]
    fn schemas_do_not_require_evidence_or_accept_caller_authority() {
        let schema = input_schema("assert_relation");
        let required = schema["required"].as_array().unwrap();
        assert!(!required.contains(&json!("evidence")));
        for field in ["caller", "actor", "origin", "permissions"] {
            assert!(schema["properties"].get(field).is_none());
        }
    }
    #[test]
    fn rest_uses_selected_compatibility_pin() {
        let schema = openapi();
        assert_eq!(schema["openapi"], "3.1.2");
        assert_eq!(schema["info"]["version"], "1.0.0");
    }
}
