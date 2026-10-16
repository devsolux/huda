//! Pure DTO/transport contracts; no external provider, model, shell, port, or secret store.
use huda::api::contracts;
use huda::relation::AssertRequest;
use serde_json::json;

#[test]
fn input_cannot_forge_origin_or_permissions() {
    let valid = json!({
        "relation": {
            "relation_type": "core:DEPENDS_ON@1",
            "roles": {
                "dependent": [{"kind": "entity", "entity_id": "entity:a"}],
                "dependency": [{"kind": "entity", "entity_id": "entity:b"}]
            }
        },
        "scope": "local",
        "idempotency_key": "contract-test",
        "expected_policy_epoch": 0
    });
    // First prove the baseline DTO is valid, so rejection cannot come from a
    // missing relation field instead of the forged authorization field.
    assert!(serde_json::from_value::<AssertRequest>(valid.clone()).is_ok());
    for (field, value) in [
        ("origin", json!({"actor": "owner"})),
        ("permissions", json!(["admin"])),
        ("actor", json!("owner")),
        ("publication_status", json!("active")),
    ] {
        let mut forged = valid.clone();
        forged[field] = value;
        assert!(serde_json::from_value::<AssertRequest>(forged).is_err());
    }
}
#[test]
fn relation_tools_and_routes_share_an_operation() {
    let assert = contracts::OPERATIONS
        .iter()
        .find(|operation| operation.tool == Some("assert_relation"))
        .unwrap();
    assert_eq!(assert.name, "assert_relation");
    assert_eq!(assert.path, "/api/v1/relations");
    let expand = contracts::OPERATIONS
        .iter()
        .find(|operation| operation.tool == Some("expand_knowledge"))
        .unwrap();
    assert_eq!(expand.name, "expand_knowledge");
    assert_eq!(expand.path, "/api/v1/knowledge/expand");
}
#[test]
fn protocol_and_product_versions_are_distinct() {
    assert_eq!(huda::VERSION, "1.2.0");
    assert_eq!(huda::MCP_VERSION, "2026-07-28");
    assert_eq!(contracts::openapi()["openapi"], "3.1.2");
}
#[test]
fn tool_catalog_exposes_no_registry_or_policy_mutation() {
    for operation in [
        "register_relation_type",
        "set_offline",
        "set_provider_policy",
        "configure_bridge",
        "backup",
        "add_principal",
    ] {
        assert!(contracts::tool_operation(operation).is_none());
    }
}
