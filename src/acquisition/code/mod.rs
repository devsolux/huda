//! Optional, explicit-input syntax adapter. No watcher, index, file scan or graph store.
pub mod parser;
pub mod semantics;
use crate::acquisition::admission::{ExtractedData, StructuredAssertion};
use crate::error::{require, Error, Result};
use crate::relation::*;
use std::collections::BTreeMap;
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "typescript",
    "tsx",
    "javascript",
    "jsx",
    "go",
    "python",
    "rust",
    "java",
    "c",
    "cpp",
    "html",
    "css",
    "csharp",
    "kotlin",
    "ruby",
    "php",
    "swift",
    "dart",
    "markdown",
    "bash",
    "json",
];

/// This conservative mapping publishes only syntactic definition membership. Parsed call,
/// inheritance and reference candidates are NOT promoted to accepted cross-file facts.
pub fn extract(source: &str, language: &str, path: &str, namespace: &str) -> Result<ExtractedData> {
    require(
        source.len() <= 262_144,
        "LIMIT_EXCEEDED",
        "Explicit code input exceeds the parser bound",
    )?;
    identifier(namespace)?;
    crate::relation::text(path, 1024)?;
    require(
        !path.starts_with('/')
            && !path.contains('\\')
            && !path.contains(':')
            && path
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != ".."),
        "INVALID_INPUT",
        "Code path must be a normalized relative path, not a filesystem capability",
    )?;
    require(
        parser::languages::get_language(language).is_some(),
        "ADAPTER_UNAVAILABLE",
        "Language is not supported by this optional parser",
    )?;
    let tree = parser::treesitter::parse_tree(source, language).map_err(|_| {
        Error::new(
            "SOURCE_FAILED",
            "Code parsing failed or exceeded its deadline",
        )
    })?;
    require(
        !tree.root_node().has_error(),
        "SOURCE_FAILED",
        "Malformed code is not qualified for automatic syntactic publication",
    )?;
    let nodes = parser::treesitter::extract_nodes_from_tree(&tree, source, language);
    let file = EntityInput {
        namespace: namespace.into(),
        kind: "code_file".into(),
        canonical_key: format!("{language}:{path}"),
        name: path.into(),
        aliases: vec![],
    };
    let file_id = file.id()?;
    let mut data = ExtractedData {
        entities: vec![file],
        assertions: vec![],
    };
    let mut seen = std::collections::BTreeSet::new();
    for node in nodes {
        require(
            data.assertions.len() < 500,
            "LIMIT_EXCEEDED",
            "Too many definitions in one code capture",
        )?;
        let name = node.qualified_name.as_deref().unwrap_or(&node.name);
        // The explicit input namespace and signature discriminator prevent label-only identity.
        let discriminator = stable_id("sig", &(&node.signature, node.start_line, node.end_line))?;
        let entity = EntityInput {
            namespace: namespace.into(),
            kind: "code_symbol".into(),
            canonical_key: format!(
                "{language}:{path}:{}:{name}:{discriminator}",
                node.node_type
            ),
            name: name.into(),
            aliases: vec![],
        };
        let id = entity.id()?;
        if !seen.insert(id.clone()) {
            continue;
        }
        let relation = RelationInput {
            relation_type: "code:DEFINED_IN@1".into(),
            roles: BTreeMap::from([
                (
                    "symbol".into(),
                    vec![RoleValue::Entity {
                        entity_id: id.clone(),
                    }],
                ),
                (
                    "file".into(),
                    vec![RoleValue::Entity {
                        entity_id: file_id.clone(),
                    }],
                ),
            ]),
            qualifiers: BTreeMap::new(),
            polarity: Polarity::Positive,
            modality: Modality::Asserted,
            condition: Default::default(),
            valid_time: None,
        };
        data.entities.push(entity);
        data.assertions.push(StructuredAssertion {
            anchor: id,
            relation,
            evidence: vec![],
            epistemic_kind: EpistemicKind::Extracted,
        });
    }
    data.validate_bounds()?;
    Ok(data)
}
/// Register this version through the ordinary privileged registry handler before admission.
pub fn definition_type() -> RelationType {
    RelationType{name:"code:DEFINED_IN".into(),version:1,description:"A syntactic symbol definition belongs to this explicitly captured code file. No runtime call or dependency inference.".into(),roles:BTreeMap::from([
        ("symbol".into(),RoleSpec{entity_kinds:["code_symbol".into()].into(),literal_types:Default::default(),min:1,max:1,ordered:false}),
        ("file".into(),RoleSpec{entity_kinds:["code_file".into()].into(),literal_types:Default::default(),min:1,max:1,ordered:false}),
    ]),qualifiers:Default::default(),traversals:vec![Traversal{from_role:"symbol".into(),to_role:"file".into(),bidirectional:false}],allow_conditions:false,allow_negation:false,allow_valid_time:false}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_filesystem_capability() {
        for p in ["../secret", "/secret", "C:/secret", "x\\y"] {
            assert!(extract("fn f() {}", "rust", p, "code-test").is_err());
        }
    }
    #[test]
    fn syntax_is_an_ordinary_relation() {
        let data = extract("fn f() {}", "rust", "src/main.rs", "code-test").unwrap();
        assert!(!data.assertions.is_empty());
        assert!(data
            .assertions
            .iter()
            .all(|a| a.relation.relation_type == "code:DEFINED_IN@1" && a.evidence.is_empty()));
    }
}
