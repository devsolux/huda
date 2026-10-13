//! The names a Rust file's `use` declarations bind, per scope (D#132).
//!
//! A call resolves by the name it writes, and Rust decides what that name means
//! through `use`: `use std::sync::Mutex; Mutex::new()` is std's, and
//! `use tokio::sync::oneshot::channel; channel()` is oneshot's, whatever other
//! `Mutex::new` or `channel` the project defines. This module answers "which
//! path does this name stand for here" so the call extractor can hand the
//! resolver the path instead of the bare name.
//!
//! Scope follows the language:
//! - a `use` binds in the module or block that holds it: a `use` inside
//!   `mod tests { … }` does not reach the file's other functions, and a file-level
//!   `use` does not reach into `mod tests` (modules do not inherit names);
//! - blocks see their enclosing block's and module's names;
//! - `use super::*` makes the parent module's names visible (the one glob
//!   followed: it names a module this file holds). Any other glob is no proof of
//!   anything and binds nothing here.
//!
//! A path's root is normalized the way the resolver reads it:
//! - `crate::…` stays; `self::…` / `super::…` are counted from the FILE's module
//!   (the inline `mod` blocks around the `use` are folded in), so `self` means
//!   "the file's module" and each leading `super` one module above it;
//! - a root that is a module this module declares (`mod util; use util::f;`,
//!   also inside a macro's body, `cfg_rt! { mod runtime; }`) becomes `self::…`;
//! - a root another `use` binds (`use crate::sync; use sync::Mutex;`) is replaced
//!   by that binding;
//! - anything else is a crate name: std's, another workspace package's, or a
//!   dependency's. The resolver tells them apart ([`UseRoot::Extern`]).

use super::super::{node_text, NamedChildAt};
use super::helpers::MAX_SUBTREE_DEPTH;
use std::collections::{HashMap, HashSet};

/// Whether a normalized path starts at this crate (`crate`/`self`/`super`) or at
/// a crate name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UseRoot {
    Project,
    Extern,
}

/// What a `use` makes of a call (see [`rewrite_call`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum UseRewrite {
    /// A path call: the callee name and the path before it, root first. A
    /// project item imported under another name (`use crate::a::f as g; g()`)
    /// is one too, of `crate::a::f`: called bare by its own name, it bound
    /// every import of that name in the file (`use crate::b::f;` beside it —
    /// batch-1 review M2).
    Path {
        name: String,
        segments: Vec<String>,
        root: UseRoot,
    },
}

/// One scope's `use` bindings.
#[derive(Default)]
struct Scope {
    /// Local name → the path it stands for, as written (root first).
    names: HashMap<String, Vec<String>>,
    /// Holds `use super::*`.
    glob_super: bool,
    /// Holds any other glob import: a name it does not bind explicitly may
    /// come from the glob, so the lookup stops here knowing nothing.
    other_glob: bool,
    /// Items defined here, by namespace: a local `fn task` is what `task()`
    /// calls even beside `use tokio::task` (a module), and it shadows every
    /// binding further out.
    value_items: HashSet<String>,
    type_items: HashSet<String>,
    /// Modules declared here (`mod x;`, `mod x { … }`, or inside an item macro).
    mods: HashSet<String>,
    /// Names of the inline `mod` blocks around this scope, outermost first.
    inline_mods: Vec<String>,
}

type Scopes = HashMap<usize, Scope>;

thread_local! {
    /// The current file's scopes, keyed by the node id of the holding
    /// `source_file` / `declaration_list` / `block`. Built on first use per file
    /// and cleared by [`reset`]: node ids are unique only within one tree, so a
    /// leaked map would bind names in an unrelated file.
    static SCOPES: std::cell::RefCell<Option<Scopes>> = const { std::cell::RefCell::new(None) };
}

/// Drop the current file's map. Called once per file before its walk.
pub(super) fn reset() {
    SCOPES.with(|s| *s.borrow_mut() = None);
}

fn is_module_scope(node: &tree_sitter::Node) -> bool {
    match node.kind() {
        "source_file" => true,
        "declaration_list" => node.parent().is_some_and(|p| p.kind() == "mod_item"),
        _ => false,
    }
}

fn with_scopes<R>(node: &tree_sitter::Node, source: &str, f: impl FnOnce(&Scopes) -> R) -> R {
    SCOPES.with(|cell| {
        if cell.borrow().is_none() {
            let mut root = *node;
            while let Some(p) = root.parent() {
                root = p;
            }
            *cell.borrow_mut() = Some(build_scopes(root, source));
        }
        f(cell.borrow().as_ref().expect("built above"))
    })
}

fn build_scopes(root: tree_sitter::Node, source: &str) -> Scopes {
    let mut scopes: Scopes = HashMap::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "use_declaration" => {
                if let Some(holder) = node.parent() {
                    let scope = scope_entry(&mut scopes, &holder, source);
                    if let Some(arg) = node.child_by_field_name("argument") {
                        collect_bindings(&arg, source, &[], scope, 0);
                    }
                }
                continue;
            }
            "extern_crate_declaration" => {
                if let (Some(holder), Some(name)) =
                    (node.parent(), node.child_by_field_name("name"))
                {
                    let krate = node_text(&name, source).to_string();
                    let local = node
                        .child_by_field_name("alias")
                        .map(|a| node_text(&a, source).to_string())
                        .unwrap_or_else(|| krate.clone());
                    let path = if krate == "self" {
                        vec!["crate".to_string()]
                    } else {
                        vec![krate]
                    };
                    scope_entry(&mut scopes, &holder, source)
                        .names
                        .insert(local, path);
                }
                continue;
            }
            "mod_item" => {
                if let (Some(holder), Some(name)) =
                    (node.parent(), node.child_by_field_name("name"))
                {
                    let name = node_text(&name, source).to_string();
                    let scope = scope_entry(&mut scopes, &holder, source);
                    scope.type_items.insert(name.clone());
                    scope.mods.insert(name);
                }
            }
            kind @ ("function_item" | "const_item" | "static_item" | "struct_item"
            | "enum_item" | "union_item" | "trait_item" | "type_item") => {
                if let (Some(holder), Some(name)) = (
                    node.parent().filter(|p| holds_items(p)),
                    node.child_by_field_name("name"),
                ) {
                    let name = node_text(&name, source).to_string();
                    let scope = scope_entry(&mut scopes, &holder, source);
                    let (value, ty) = item_namespaces(kind);
                    if value {
                        scope.value_items.insert(name.clone());
                    }
                    if ty {
                        scope.type_items.insert(name);
                    }
                }
            }
            "macro_invocation" => {
                // An item macro's body declares modules the tree cannot see
                // (`cfg_rt! { pub mod runtime; }`): read `mod <name>` off its
                // tokens.
                if let Some(holder) = node.parent().filter(|p| is_module_scope(p)) {
                    let items = macro_declared_items(&node, source);
                    if !items.is_empty() {
                        let scope = scope_entry(&mut scopes, &holder, source);
                        for (kind, name) in items {
                            let (value, ty) = item_namespaces(kind);
                            if kind == "mod_item" {
                                scope.mods.insert(name.clone());
                            }
                            if value {
                                scope.value_items.insert(name.clone());
                            }
                            if ty {
                                scope.type_items.insert(name);
                            }
                        }
                    }
                }
                continue;
            }
            _ => {}
        }
        for i in (0..node.named_child_count()).rev() {
            if let Some(c) = node.named_child_at(i) {
                stack.push(c);
            }
        }
    }
    scopes
}

fn scope_entry<'a>(
    scopes: &'a mut Scopes,
    holder: &tree_sitter::Node,
    source: &str,
) -> &'a mut Scope {
    scopes.entry(holder.id()).or_insert_with(|| Scope {
        inline_mods: super::rust::enclosing_inline_mods(holder, source),
        ..Scope::default()
    })
}

/// Whether items directly under `node` are scope items (not an `impl`'s or a
/// `trait`'s associated items).
fn holds_items(node: &tree_sitter::Node) -> bool {
    matches!(node.kind(), "source_file" | "block") || is_module_scope(node)
}

/// (value namespace, type namespace) of an item kind. A struct is both: a tuple
/// or unit struct is also a value.
fn item_namespaces(kind: &str) -> (bool, bool) {
    match kind {
        "function_item" | "const_item" | "static_item" => (true, false),
        "struct_item" => (true, true),
        _ => (false, true),
    }
}

/// `<keyword> <ident>` item declarations in an item macro's body
/// (`cfg_rt! { pub mod runtime; pub fn spawn() {} }`), as (item kind, name).
fn macro_declared_items(node: &tree_sitter::Node, source: &str) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut stack = vec![*node];
    let mut seen = 0usize;
    while let Some(n) = stack.pop() {
        seen += 1;
        if seen > 4096 {
            break;
        }
        let mut prev: Option<&'static str> = None;
        for i in 0..n.child_count() {
            let Some(c) = n.child(i) else { continue };
            if c.kind() == "token_tree" {
                stack.push(c);
                prev = None;
                continue;
            }
            let text = node_text(&c, source);
            if let Some(kind) = prev {
                if c.kind() == "identifier" {
                    out.push((kind, text.to_string()));
                }
            }
            prev = match text {
                "mod" => Some("mod_item"),
                "fn" => Some("function_item"),
                "const" => Some("const_item"),
                "static" => Some("static_item"),
                "struct" => Some("struct_item"),
                "enum" => Some("enum_item"),
                "union" => Some("union_item"),
                "trait" => Some("trait_item"),
                "type" => Some("type_item"),
                _ => None,
            };
        }
    }
    out
}

/// Record every name a use clause binds, with the path written before it.
fn collect_bindings(
    node: &tree_sitter::Node,
    source: &str,
    prefix: &[String],
    scope: &mut Scope,
    depth: usize,
) {
    if depth > MAX_SUBTREE_DEPTH {
        return;
    }
    let path_of = |n: Option<tree_sitter::Node>| -> Vec<String> {
        let mut segs = prefix.to_vec();
        if let Some(p) = n {
            segs.extend(
                node_text(&p, source)
                    .split("::")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
            );
        }
        segs
    };
    match node.kind() {
        "use_as_clause" => {
            let (Some(path), Some(alias)) = (
                node.child_by_field_name("path"),
                node.child_by_field_name("alias"),
            ) else {
                return;
            };
            let alias = node_text(&alias, source);
            let full = path_of(Some(path));
            if alias == "_" || full.is_empty() {
                return;
            }
            let full = if full.last().is_some_and(|s| s == "self") {
                full[..full.len() - 1].to_vec()
            } else {
                full
            };
            if !full.is_empty() {
                scope.names.insert(alias.to_string(), full);
            }
        }
        "use_wildcard" => {
            // `use super::*` only: the parent module's names become visible.
            let text = node_text(node, source).trim_end_matches('*');
            let mut segs = prefix.to_vec();
            segs.extend(
                text.split("::")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
            );
            if segs == ["super"] {
                scope.glob_super = true;
            } else {
                scope.other_glob = true;
            }
        }
        "use_list" => {
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child_at(i) {
                    collect_bindings(&c, source, prefix, scope, depth + 1);
                }
            }
        }
        "scoped_use_list" => {
            let segs = path_of(node.child_by_field_name("path"));
            if let Some(list) = node.child_by_field_name("list") {
                collect_bindings(&list, source, &segs, scope, depth + 1);
            }
        }
        "self" => {
            // `use a::b::{self, C}` binds `b`.
            if let Some(last) = prefix.last() {
                if !matches!(last.as_str(), "crate" | "self" | "super") {
                    scope.names.insert(last.clone(), prefix.to_vec());
                }
            }
        }
        "scoped_identifier" | "identifier" | "crate" | "super" => {
            let full = path_of(Some(*node));
            if let Some(last) = full.last() {
                if !matches!(last.as_str(), "crate" | "self" | "super" | "*") {
                    scope.names.insert(last.clone(), full.clone());
                }
            }
        }
        _ => {}
    }
}

/// The path `name` stands for at `node`, normalized (see the module doc).
/// `value` looks the name up as a value (a bare call's callee), else as a type
/// or module (a path's first segment).
pub(super) fn binding(
    node: &tree_sitter::Node,
    source: &str,
    name: &str,
    value: bool,
) -> Option<(Vec<String>, UseRoot)> {
    with_scopes(node, source, |scopes| {
        let (path, scope_node) = lookup(scopes, node, name, false, value)?;
        normalize(scopes, &scope_node, path, 0)
    })
}

/// Normalize a `use` path written in the scope `scope_node` (see the module doc).
fn normalize(
    scopes: &Scopes,
    scope_node: &tree_sitter::Node,
    path: Vec<String>,
    depth: usize,
) -> Option<(Vec<String>, UseRoot)> {
    let inline_mods = match scopes.get(&scope_node.id()) {
        Some(scope) => scope.inline_mods.clone(),
        None => Vec::new(),
    };
    let first = path.first()?.clone();
    match first.as_str() {
        "crate" => Some((path, UseRoot::Project)),
        "self" | "super" => Some((relative_to_file(&path, &inline_mods)?, UseRoot::Project)),
        _ if declares_mod(scopes, scope_node, &first) => {
            let mut rel = vec!["self".to_string()];
            rel.extend(path);
            Some((relative_to_file(&rel, &inline_mods)?, UseRoot::Project))
        }
        _ => {
            // A root another `use` binds, looked up from the scope of this one
            // (a binding never answers for its own root: `use foo;`).
            if depth < 3 {
                if let Some((base_path, base_scope)) =
                    lookup(scopes, scope_node, &first, true, false)
                {
                    if base_path != path {
                        if let Some((mut base, root)) =
                            normalize(scopes, &base_scope, base_path, depth + 1)
                        {
                            base.extend(path.into_iter().skip(1));
                            return Some((base, root));
                        }
                    }
                }
            }
            Some((path, UseRoot::Extern))
        }
    }
}

/// A `use` declaration's own path (`segments`, the imported name last),
/// normalized like a binding: what the import names, for its edge's metadata.
pub(super) fn normalize_use_path(
    use_node: &tree_sitter::Node,
    source: &str,
    segments: &[String],
) -> Option<(Vec<String>, UseRoot)> {
    let scope_node = use_node.parent()?;
    with_scopes(use_node, source, |scopes| {
        normalize(scopes, &scope_node, segments.to_vec(), 0)
    })
}

/// Where the lookup of a name ended (see [`lookup_origin`]).
enum Found<'t> {
    /// A `use` binding: the path it writes and the scope holding it.
    Use(Vec<String>, tree_sitter::Node<'t>),
    /// An item defined in a scope the name is visible from.
    Local,
    /// A glob may bind it: nothing is known.
    Glob,
    /// Neither: a prelude name, a crate name, or an item the tree cannot see.
    Nothing,
}

/// Walk out from `node` (a call, or with `from_scope` the scope a `use` sits
/// in) to the scope holding a binding of `name`: blocks, then the module; past
/// a module only through `use super::*`.
fn lookup<'t>(
    scopes: &Scopes,
    node: &tree_sitter::Node<'t>,
    name: &str,
    from_scope: bool,
    value: bool,
) -> Option<(Vec<String>, tree_sitter::Node<'t>)> {
    match lookup_origin(scopes, node, name, from_scope, value) {
        Found::Use(path, scope) => Some((path, scope)),
        _ => None,
    }
}

fn lookup_origin<'t>(
    scopes: &Scopes,
    node: &tree_sitter::Node<'t>,
    name: &str,
    from_scope: bool,
    value: bool,
) -> Found<'t> {
    let mut cur = if from_scope {
        Some(*node)
    } else {
        node.parent()
    };
    while let Some(n) = cur {
        let scope = scopes.get(&n.id());
        if let Some(scope) = scope {
            let items = if value {
                &scope.value_items
            } else {
                &scope.type_items
            };
            if items.contains(name) {
                return Found::Local;
            }
            if let Some(path) = scope.names.get(name) {
                return Found::Use(path.clone(), n);
            }
            if scope.other_glob {
                return Found::Glob;
            }
        }
        if is_module_scope(&n) && !scope.is_some_and(|s| s.glob_super) {
            return Found::Nothing;
        }
        cur = n.parent();
    }
    Found::Nothing
}

/// What a type (or a type path's first segment) written at `node` names, as the
/// receiver typing in `rust_receiver.rs` reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TypeOrigin {
    /// This crate's (`crate::`/`self::`/`super::`, a module or item of the file).
    Project,
    /// A path rooted at a crate name: std's, a dependency's or another
    /// workspace package's (the resolver tells them apart).
    Extern(String),
    /// Written with no `use` and no local item: a prelude name, or an item the
    /// tree cannot see (a macro's).
    Unbound,
    /// A glob import may bind it: nothing is known.
    Unknown,
}

/// The origin of the type path `segments` written at `node`: its first segment
/// looked up in the type namespace like a path call's.
pub(super) fn type_origin(
    node: &tree_sitter::Node,
    source: &str,
    segments: &[String],
) -> TypeOrigin {
    let Some(first) = segments.first() else {
        return TypeOrigin::Unknown;
    };
    if matches!(first.as_str(), "crate" | "self" | "super") {
        return TypeOrigin::Project;
    }
    with_scopes(node, source, |scopes| {
        match lookup_origin(scopes, node, first, false, false) {
            Found::Use(path, scope) => match normalize(scopes, &scope, path, 0) {
                Some((_, UseRoot::Project)) => TypeOrigin::Project,
                Some((path, UseRoot::Extern)) => match path.first() {
                    Some(root) => TypeOrigin::Extern(root.clone()),
                    None => TypeOrigin::Unknown,
                },
                None => TypeOrigin::Unknown,
            },
            Found::Local => TypeOrigin::Project,
            Found::Glob => TypeOrigin::Unknown,
            // `a::T` with no `use` of `a`: a crate name (`tempfile::TempDir`).
            Found::Nothing if segments.len() > 1 => TypeOrigin::Extern(first.clone()),
            Found::Nothing => TypeOrigin::Unbound,
        }
    })
}

/// The full path the type path `segments` written at `node` stands for, when
/// its first segment is `crate` or a name a `use` binds (normalized like a
/// binding: `use crate::loom::sync::Mutex;` → `crate::loom::sync::Mutex`), or
/// a crate name no `use` binds (`tokio::sync::Semaphore` as written).
pub(super) fn type_path(
    node: &tree_sitter::Node,
    source: &str,
    segments: &[String],
) -> Option<Vec<String>> {
    let (first, rest) = segments.split_first()?;
    match first.as_str() {
        "crate" => return Some(segments.to_vec()),
        "self" | "super" | "Self" => return None,
        _ => {}
    }
    with_scopes(node, source, |scopes| {
        match lookup_origin(scopes, node, first, false, false) {
            Found::Use(path, scope) => {
                let (mut base, _) = normalize(scopes, &scope, path, 0)?;
                base.extend(rest.iter().cloned());
                Some(base)
            }
            Found::Nothing if !rest.is_empty() => Some(segments.to_vec()),
            _ => None,
        }
    })
}

/// The origin of the value `name` (a bare call's callee) at `node`, as
/// [`type_origin`] reads a type.
pub(super) fn value_origin(node: &tree_sitter::Node, source: &str, name: &str) -> TypeOrigin {
    with_scopes(node, source, |scopes| {
        match lookup_origin(scopes, node, name, false, true) {
            Found::Use(path, scope) => match normalize(scopes, &scope, path, 0) {
                Some((_, UseRoot::Project)) => TypeOrigin::Project,
                Some((path, UseRoot::Extern)) => match path.first() {
                    Some(root) => TypeOrigin::Extern(root.clone()),
                    None => TypeOrigin::Unknown,
                },
                None => TypeOrigin::Unknown,
            },
            Found::Local => TypeOrigin::Project,
            Found::Glob => TypeOrigin::Unknown,
            Found::Nothing => TypeOrigin::Unbound,
        }
    })
}

/// The last segment of the path a `use` binds `name` to at `node`, when it
/// renames a type (`use a::Widget as W;` → `Widget`).
pub(super) fn type_binding_name(
    node: &tree_sitter::Node,
    source: &str,
    name: &str,
) -> Option<String> {
    with_scopes(node, source, |scopes| {
        match lookup_origin(scopes, node, name, false, false) {
            Found::Use(path, _) => path.last().cloned(),
            _ => None,
        }
    })
}

/// Whether the module holding `scope_node` (or a block between) declares `mod name`.
fn declares_mod(scopes: &Scopes, scope_node: &tree_sitter::Node, name: &str) -> bool {
    let mut cur = Some(*scope_node);
    while let Some(n) = cur {
        if scopes.get(&n.id()).is_some_and(|s| s.mods.contains(name)) {
            return true;
        }
        if is_module_scope(&n) {
            return false;
        }
        cur = n.parent();
    }
    false
}

/// `self::…` / `super::…` written inside `inline_mods`, re-counted from the
/// file's module: `self` is the file's module, each leading `super` one above.
fn relative_to_file(path: &[String], inline_mods: &[String]) -> Option<Vec<String>> {
    let supers = path.iter().take_while(|s| *s == "super").count();
    let skip = supers.max(usize::from(path.first().is_some_and(|s| s == "self")));
    let tail = &path[skip..];
    let mut out = Vec::new();
    if supers <= inline_mods.len() {
        out.push("self".to_string());
        out.extend(inline_mods[..inline_mods.len() - supers].iter().cloned());
    } else {
        out.extend(std::iter::repeat_n(
            "super".to_string(),
            supers - inline_mods.len(),
        ));
    }
    out.extend(tail.iter().cloned());
    Some(out)
}

/// The leftmost identifier of a Rust callee path, when the path opens with a
/// plain name (not `crate`/`self`/`super`/`Self`, a leading `::`, or `<T as Tr>`).
pub(super) fn leftmost_name<'t>(function: &tree_sitter::Node<'t>) -> Option<tree_sitter::Node<'t>> {
    let mut cur = *function;
    for _ in 0..MAX_SUBTREE_DEPTH {
        match cur.kind() {
            "identifier" | "type_identifier" => return Some(cur),
            "scoped_identifier" | "scoped_type_identifier" => {
                cur = cur.child_by_field_name("path")?
            }
            "generic_type" => cur = cur.child_by_field_name("type")?,
            "generic_function" => cur = cur.child_by_field_name("function")?,
            _ => return None,
        }
    }
    None
}

/// What the file's `use` declarations make of a Rust call: `callee` called bare
/// (`path` None) or through `path` whose first segment is `first` as written.
/// None when no `use` binds the name the call starts with, or the binding
/// changes nothing (a project item called by its own name).
pub(super) fn rewrite_call(
    node: &tree_sitter::Node,
    source: &str,
    callee: &str,
    path: Option<&[String]>,
) -> Option<UseRewrite> {
    match path {
        None => {
            let (bound, root) = binding(node, source, callee, true)?;
            let (name, segments) = bound.split_last()?;
            match root {
                UseRoot::Project if name == callee => None,
                UseRoot::Project | UseRoot::Extern => Some(UseRewrite::Path {
                    name: name.clone(),
                    segments: segments.to_vec(),
                    root,
                }),
            }
        }
        Some(path) => {
            let (first, rest) = path.split_first()?;
            let (mut segments, root) = binding(node, source, first, false)?;
            segments.extend(rest.iter().cloned());
            Some(UseRewrite::Path {
                name: callee.to_string(),
                segments,
                root,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_call(src: &str, callee_text: &str) -> Option<(Vec<String>, UseRoot)> {
        let tree = crate::acquisition::code::parser::treesitter::parse_tree(src, "rust").unwrap();
        reset();
        // The first identifier spelled `callee_text` inside a call.
        let mut stack = vec![tree.root_node()];
        while let Some(n) = stack.pop() {
            if n.kind() == "call_expression" {
                let f = n.child_by_field_name("function").unwrap();
                if let Some(id) = leftmost_name(&f) {
                    if node_text(&id, src) == callee_text {
                        let bare = f.kind() == "identifier";
                        let out = binding(&n, src, callee_text, bare);
                        reset();
                        return out;
                    }
                }
            }
            for i in (0..n.named_child_count()).rev() {
                stack.push(n.named_child_at(i).unwrap());
            }
        }
        panic!("no call through {callee_text}");
    }

    fn p(s: &str) -> Vec<String> {
        s.split("::").map(String::from).collect()
    }

    #[test]
    fn use_bindings_by_shape() {
        use UseRoot::{Extern, Project};
        // (source, name the call opens with, expected path and root)
        type Case<'a> = (&'a str, &'a str, Option<(&'a str, UseRoot)>);
        let cases: &[Case] = &[
            ("use std::sync::Mutex;\nfn f() { Mutex::new(); }", "Mutex", Some(("std::sync::Mutex", Extern))),
            ("use ::std::mem::swap;\nfn f() { swap(); }", "swap", Some(("std::mem::swap", Extern))),
            ("use a::b::C as D;\nfn f() { D::new(); }", "D", Some(("a::b::C", Extern))),
            ("use a::{b::C, d::{E, F as G}};\nfn f() { G(); }", "G", Some(("a::d::F", Extern))),
            ("use a::{b::C, d::{E, F as G}};\nfn f() { E(); }", "E", Some(("a::d::E", Extern))),
            ("use tokio::sync::{self, oneshot};\nfn f() { sync::x(); }", "sync", Some(("tokio::sync", Extern))),
            ("use a::b::*;\nfn f() { C::new(); }", "C", None),
            ("use a::Tr as _;\nfn f() { Tr::x(); }", "Tr", None),
            ("use crate::sync::Mutex;\nfn f() { Mutex::new(); }", "Mutex", Some(("crate::sync::Mutex", Project))),
            ("use super::x::Y;\nfn f() { Y::new(); }", "Y", Some(("super::x::Y", Project))),
            ("use self::x::Y;\nfn f() { Y::new(); }", "Y", Some(("self::x::Y", Project))),
            // Inside `mod m`: `super` is the file's module, `self` is `m`.
            ("mod m {\n use super::x::Y;\n fn f() { Y::new(); }\n}", "Y", Some(("self::x::Y", Project))),
            ("mod m {\n use self::x::Y;\n fn f() { Y::new(); }\n}", "Y", Some(("self::m::x::Y", Project))),
            // A file-level use does not reach into `mod tests`...
            ("use std::sync::Mutex;\nmod tests {\n fn t() { Mutex::new(); }\n}", "Mutex", None),
            // ...unless it holds `use super::*`.
            ("use std::sync::Mutex;\nmod tests {\n use super::*;\n fn t() { Mutex::new(); }\n}", "Mutex", Some(("std::sync::Mutex", Extern))),
            // A `use` in `mod tests` binds only there.
            ("use crate::sync::Mutex;\nmod tests {\n use std::sync::Mutex;\n fn t() { Mutex::new(); }\n}", "Mutex", Some(("std::sync::Mutex", Extern))),
            ("fn f() {\n use std::sync::Mutex;\n { Mutex::new(); }\n}", "Mutex", Some(("std::sync::Mutex", Extern))),
            // A root this module declares is a module of the file.
            ("mod util;\nuse util::helper;\nfn f() { helper(); }", "helper", Some(("self::util::helper", Project))),
            ("cfg_rt! { pub mod runtime; }\nuse runtime::Builder;\nfn f() { Builder::new(); }", "Builder", Some(("self::runtime::Builder", Project))),
            // A root another use binds.
            ("use crate::sync;\nuse sync::Mutex;\nfn f() { Mutex::new(); }", "Mutex", Some(("crate::sync::Mutex", Project))),
            ("extern crate my_crate as mc;\nuse mc::sync::oneshot;\nfn f() { oneshot::channel(); }", "oneshot", Some(("my_crate::sync::oneshot", Extern))),
            ("extern crate self as mc;\nuse mc::sync::oneshot;\nfn f() { oneshot::channel(); }", "oneshot", Some(("crate::sync::oneshot", Project))),
            ("pub use std::sync::Mutex;\nfn f() { Mutex::new(); }", "Mutex", Some(("std::sync::Mutex", Extern))),
            ("fn f() { Mutex::new(); }", "Mutex", None),
            // A local item of the call's namespace wins over a `use` of the name.
            ("use tokio::task;\nasync fn task() {}\nfn f() { task(); }", "task", None),
            ("use tokio::task;\nasync fn task() {}\nfn f() { task::spawn(); }", "task", Some(("tokio::task", Extern))),
            ("use core::ptr;\nmod tests {\n use super::*;\n fn ptr() {}\n fn t() { ptr(); }\n}", "ptr", None),
            ("cfg_rt! { pub fn spawn() {} }\nuse tokio::spawn;\nfn f() { spawn(); }", "spawn", None),
            // A glob between the call and the binding may be where the name comes from.
            ("use crate::a::Semaphore;\nfn f() {\n use crate::b::*;\n Semaphore::new();\n}", "Semaphore", None),
        ];
        let mut bad = Vec::new();
        for (src, name, want) in cases {
            let got = at_call(src, name);
            let want = want.map(|(path, root)| (p(path), root));
            if got != want {
                bad.push(format!("{src:?}: got {got:?}, want {want:?}"));
            }
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }
}
