//! Dart-specific extraction. Two distinct AST shapes:
//! - imports: `import_or_export` → drill through `library_import` →
//!   `import_specification` → `configurable_uri`/`uri` → `string_literal`,
//!   then strip `dart:` / `package:` / relative path prefix to a bare module name.
//! - calls: `method_invocation` whose `function` field is the callee; method-style
//!   chains (`obj.transform()`) take the last identifier of the assignable / cascade
//!   selector. A call on a call's result (`getFn()()`) keeps the older flat
//!   `identifier` + `selector(argument_part)` form.

use super::super::{node_text, NamedChildAt};
use super::helpers::MAX_SUBTREE_DEPTH;
use super::ParsedRelation;
use crate::acquisition::code::semantics::{REL_CALLS, REL_IMPORTS};

/// Extract Dart import targets from `import_or_export` nodes.
/// AST: import_or_export -> library_import -> import_specification -> configurable_uri/uri -> string_literal
pub(super) fn extract_dart_imports(
    node: &tree_sitter::Node,
    source: &str,
    results: &mut Vec<ParsedRelation>,
) {
    fn find_uri_string(node: &tree_sitter::Node, source: &str) -> Option<String> {
        find_uri_string_inner(node, source, 0)
    }
    fn find_uri_string_inner(
        node: &tree_sitter::Node,
        source: &str,
        depth: usize,
    ) -> Option<String> {
        if depth > MAX_SUBTREE_DEPTH {
            return None;
        }
        if node.kind() == "string_literal" {
            let text = node_text(node, source);
            // Strip quotes: 'dart:async' -> dart:async
            let trimmed = text.trim_matches('\'').trim_matches('"');
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child_at(i) {
                if let Some(result) = find_uri_string_inner(&child, source, depth + 1) {
                    return Some(result);
                }
            }
        }
        None
    }

    if let Some(uri) = find_uri_string(node, source) {
        // Extract meaningful name: 'dart:async' -> 'async', 'package:foo/bar.dart' -> 'bar'
        let import_name = if let Some(rest) = uri.strip_prefix("dart:") {
            rest.to_string()
        } else if let Some(rest) = uri.strip_prefix("package:") {
            // package:foo/bar.dart -> last segment without .dart
            rest.rsplit('/')
                .next()
                .unwrap_or(rest)
                .trim_end_matches(".dart")
                .to_string()
        } else {
            // Relative import: 'src/utils.dart' -> 'utils'
            uri.rsplit('/')
                .next()
                .unwrap_or(&uri)
                .trim_end_matches(".dart")
                .to_string()
        };
        if !import_name.is_empty() {
            results.push(ParsedRelation {
                source_name: "<module>".into(),
                target_name: import_name,
                relation: REL_IMPORTS.into(),
                metadata: None,
                source_language: String::new(),
                source_line: None,
            });
        }
    }
}

/// Extract a Dart function/method call from a `method_invocation` node. The
/// callee is its `function` field:
///   - `foo()` / `Widget(...)` / `gen<int>()` → `identifier`                → `foo`
///   - `obj.run()` / `obj?.run()` / `x!.run()` / `super.run()` / `a.b.run()`
///     → `(un)conditional_assignable_selector`, its last direct identifier → `run`
///   - `obj..run()` → `cascade_selector`; `.parse()` → `dot_shorthand`      → name
///   - `foo(bar())` → outer invocation → `foo`; inner invocation → `bar` (both)
///
/// Any other callee (`(f)()`) names no symbol and is skipped.
pub(super) fn extract_dart_call_from_invocation(
    node: &tree_sitter::Node,
    source: &str,
    scope: &str,
    results: &mut Vec<ParsedRelation>,
) {
    let Some(function) = node.child_by_field_name("function") else {
        return;
    };
    let callee = match function.kind() {
        "identifier" | "type_identifier" => Some(node_text(&function, source).to_string()),
        "unconditional_assignable_selector"
        | "conditional_assignable_selector"
        | "cascade_selector"
        | "dot_shorthand" => (0..function.named_child_count())
            .filter_map(|i| function.named_child_at(i))
            .rfind(|c| c.kind() == "identifier")
            .map(|id| node_text(&id, source).to_string()),
        _ => None,
    };
    push_dart_call(callee, scope, results);
}

/// Extract a Dart call from the residual flat form: a `selector` node that carries
/// an `argument_part` (i.e. `(...)`), whose callee is the selector's preceding
/// named sibling. tree-sitter-dart-orchard 0.7 keeps this shape only for a call on
/// a call's result — `getFn()()` → `[identifier getFn, selector(args),
/// selector(args)]` → `getFn` — every other call is a `method_invocation`.
pub(super) fn extract_dart_call_from_selector(
    node: &tree_sitter::Node,
    source: &str,
    scope: &str,
    results: &mut Vec<ParsedRelation>,
) {
    // A selector is a call only when it directly contains an argument_part.
    let is_call = (0..node.named_child_count())
        .filter_map(|i| node.named_child_at(i))
        .any(|c| c.kind() == "argument_part");
    if !is_call {
        return;
    }

    let prev = match node.prev_named_sibling() {
        Some(p) => p,
        None => return,
    };
    let callee = match prev.kind() {
        // Method call: the preceding selector wraps `.name` / `?.name`.
        "selector" => assignable_selector_name(&prev, source),
        // Plain function / constructor call: `foo(...)` / `Widget(...)`.
        "identifier" | "type_identifier" => Some(node_text(&prev, source).to_string()),
        _ => None,
    };
    push_dart_call(callee, scope, results);
}

fn push_dart_call(callee: Option<String>, scope: &str, results: &mut Vec<ParsedRelation>) {
    if let Some(callee) = callee.filter(|c| !c.is_empty()) {
        results.push(ParsedRelation {
            source_name: scope.to_string(),
            target_name: callee,
            relation: REL_CALLS.into(),
            metadata: None,
            source_language: String::new(),
            source_line: None,
        });
    }
}

/// Pull the method name out of a `selector` that wraps an
/// `unconditional_assignable_selector` / `conditional_assignable_selector`
/// (`.name` / `?.name`) → the inner `identifier` text.
fn assignable_selector_name(selector: &tree_sitter::Node, source: &str) -> Option<String> {
    for i in 0..selector.named_child_count() {
        let inner = selector.named_child_at(i)?;
        if matches!(
            inner.kind(),
            "unconditional_assignable_selector" | "conditional_assignable_selector"
        ) {
            for j in 0..inner.named_child_count() {
                if let Some(id) = inner.named_child_at(j) {
                    if id.kind() == "identifier" {
                        return Some(node_text(&id, source).to_string());
                    }
                }
            }
        }
    }
    None
}
