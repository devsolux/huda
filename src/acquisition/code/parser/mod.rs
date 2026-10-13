pub mod lang_config;
pub mod languages;
pub mod relations;
pub mod treesitter;

/// Safely extract the text corresponding to a tree-sitter node from the source string.
/// Returns `""` if the byte range is out of bounds.
pub fn node_text<'a>(node: &tree_sitter::Node, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

/// tree-sitter 0.26+ indexes named children by `u32` while `named_child_count()`
/// still returns `usize`; this keeps `0..named_child_count()` walks in one index
/// type. An index past `u32::MAX` cannot name a child, so it yields `None`.
pub(crate) trait NamedChildAt<'tree> {
    fn named_child_at(&self, index: usize) -> Option<tree_sitter::Node<'tree>>;
}

impl<'tree> NamedChildAt<'tree> for tree_sitter::Node<'tree> {
    fn named_child_at(&self, index: usize) -> Option<tree_sitter::Node<'tree>> {
        u32::try_from(index).ok().and_then(|i| self.named_child(i))
    }
}

/// The name a Rust impl block's type is recorded under: its generic arguments
/// dropped, then its last path segment (`crate::a::List<L, L::Target>` →
/// `List`). Single source of truth for the node walk (method qualified names),
/// the relation walk (`self.m()` / `Self::m()` payloads) and trait-impl
/// heritage, which must agree for a method filter to find anything. The
/// arguments go first because they may hold paths themselves: splitting on
/// `::` first named that impl `Target>`, and a payload that kept them
/// (`Gen<T>`) matched no method at all.
pub(crate) fn rust_impl_type_name(type_text: &str) -> String {
    rust_type_path(type_text).pop().unwrap_or_default()
}

/// The path segments of a Rust type as written, its generic arguments dropped
/// (`std::io::Error` → [std, io, Error]; `List<L, L::Target>` → [List]).
pub(crate) fn rust_type_path(type_text: &str) -> Vec<String> {
    let mut bare = String::with_capacity(type_text.len());
    let mut depth = 0usize;
    let mut prev = '\0';
    for c in type_text.chars() {
        // The `>` of `->` (`Box<dyn Fn() -> u8>`) closes nothing.
        let arrow = c == '>' && prev == '-';
        match c {
            '<' => depth += 1,
            '>' if depth > 0 && !arrow => depth -= 1,
            _ if depth == 0 => bare.push(c),
            _ => {}
        }
        prev = c;
    }
    bare.split("::").map(|s| s.trim().to_string()).collect()
}

/// Recognize an Express/Fastify/Koa-style HTTP route registration call
/// (`app|router|server|fastify.METHOD(path, ...)`) and return its (METHOD, path).
/// Single source of truth for the recognized receiver objects + HTTP-method map,
/// so route-edge extraction (`relations::routes`) and inline-handler node
/// materialization (`treesitter` + the relations walker) can never drift.
pub(crate) fn express_route_method_path(
    call: &tree_sitter::Node,
    source: &str,
) -> Option<(&'static str, String)> {
    let function = call.child_by_field_name("function")?;
    if function.kind() != "member_expression" {
        return None;
    }
    let object = function.child_by_field_name("object")?;
    let property = function.child_by_field_name("property")?;
    if !matches!(
        node_text(&object, source),
        "app" | "router" | "server" | "fastify"
    ) {
        return None;
    }
    let method = match node_text(&property, source) {
        "get" => "GET",
        "post" => "POST",
        "put" => "PUT",
        "delete" => "DELETE",
        "patch" => "PATCH",
        "use" => "USE",
        _ => return None,
    };
    let args = call.child_by_field_name("arguments")?;
    let first = args.named_child(0)?;
    let path = node_text(&first, source)
        .trim_matches(|c| c == '\'' || c == '"')
        .to_string();
    Some((method, path))
}

/// Build the resolution-stable node name for an inline route handler from its
/// method + path, e.g. ("GET", "/api/users") → "GET /api/users". Returns None
/// when `path` isn't a concrete route path, so callers keep the legacy
/// `<module>` attribution instead of emitting an edge to a handler node the
/// materialization step won't create.
pub(crate) fn synthetic_route_handler_name(method: &str, path: &str) -> Option<String> {
    let path = path.trim_matches(|c| c == '\'' || c == '"');
    if !path.starts_with('/') {
        return None;
    }
    Some(format!("{} {}", method, path))
}

/// If `node` is the inline arrow / function-expression handler of an HTTP route
/// registration (the LAST argument of an `app|router|server|fastify.METHOD(path,
/// ...)` call), return the per-occurrence handler node name "METHOD path#Lstart"
/// (the start-line suffix disambiguates multiple inline handlers for the SAME
/// route in one file). Used by the node extractor (Phase 1) and the relations
/// walker (Phase 2) so the handler node, its scoped calls, and the routes_to edge
/// all share one name.
pub(crate) fn route_handler_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    if !matches!(
        node.kind(),
        "arrow_function" | "function_expression" | "function"
    ) {
        return None;
    }
    let args = node.parent().filter(|p| p.kind() == "arguments")?;
    let n = args.named_child_count();
    if n < 2 {
        return None; // need at least (path, handler)
    }
    // Must be the LAST argument — the handler, not a middleware arrow.
    if args.named_child_at(n - 1)?.id() != node.id() {
        return None;
    }
    let call = args.parent().filter(|p| p.kind() == "call_expression")?;
    let (method, path) = express_route_method_path(&call, source)?;
    let base = synthetic_route_handler_name(method, &path)?;
    // Append the handler's start line to disambiguate multiple inline handlers
    // for the SAME method+path in one file (valid: conditional / overloaded
    // registration). Without it every same-route handler collapses onto one
    // synthetic name, so name-based edge resolution cross-links their scoped calls
    // and fans routes_to into a cartesian product (src{N}×tgt{N}). The line keeps
    // node identity, the handler's calls, and the routes_to edge 1:1.
    // find_routes_by_path matches on the metadata `$.path` (storage/queries/
    // routes.rs), not the node name, so trace / route lookup is unaffected.
    Some(format!("{}#L{}", base, node.start_position().row + 1))
}

/// A JS/TS function literal assigned to a named member, the way CommonJS code
/// defines most of its functions: `res.send = function send(body) {}`,
/// `View.prototype.lookup = function () {}`, `exports.f = () => {}`,
/// `module.exports.f = …` (express 4.21.2: 70 of the 107 functions in `lib/`).
/// Returns `(name, qualified_name, node_type, assignment)`: the property name;
/// the member path with `prototype` segments dropped and `module.exports`
/// spelled `exports`; `"method"` for a prototype member, else `"function"`.
/// Every segment must be a plain identifier: `obj[k] = …` and `f().x = …` name
/// no stable member, and neither does `this.x = …` — in a class method it
/// replaces the method at run time (hono's `this.match = …` drew the class's own
/// `match` calls), in a constructor function it names each instance's slot.
/// Shared by the node extractor and the
/// relations walker so a function's node and its calls' scope agree
/// (tasks/specs/js-ts-assigned-functions.md, D7).
pub(crate) fn js_member_assigned_function<'t>(
    node: &tree_sitter::Node<'t>,
    source: &str,
) -> Option<(String, String, &'static str, tree_sitter::Node<'t>)> {
    if !matches!(node.kind(), "arrow_function" | "function_expression") {
        return None;
    }
    let assign = node
        .parent()
        .filter(|p| p.kind() == "assignment_expression")?;
    if assign.child_by_field_name("right")?.id() != node.id() {
        return None;
    }
    let (name, qualified, kind) = js_member_path(&assign.child_by_field_name("left")?, source)?;
    Some((name, qualified, kind, assign))
}

/// Every member one function literal is assigned to, innermost first:
/// `res.set = res.header = function header() {}` names it `res.header` and
/// `res.set` (express: `req.get`/`req.header`, `res.type`/`res.contentType`).
/// Each carries the outermost assignment, the statement that holds them all.
pub(crate) fn js_member_assignment_chain<'t>(
    node: &tree_sitter::Node<'t>,
    source: &str,
) -> Vec<(String, String, &'static str, tree_sitter::Node<'t>)> {
    let Some((name, qualified, kind, mut outer)) = js_member_assigned_function(node, source) else {
        return Vec::new();
    };
    let mut names = vec![(name, qualified, kind)];
    while let Some(parent) = outer
        .parent()
        .filter(|p| p.kind() == "assignment_expression")
        .filter(|p| {
            p.child_by_field_name("right")
                .is_some_and(|r| r.id() == outer.id())
        })
    {
        match parent
            .child_by_field_name("left")
            .and_then(|l| js_member_path(&l, source))
        {
            Some(entry) => names.push(entry),
            None => break,
        }
        outer = parent;
    }
    names
        .into_iter()
        .map(|(n, q, k)| (n, q, k, outer))
        .collect()
}

/// `(name, qualified_name, node_type)` of a plain member path used as an
/// assignment target (see [`js_member_assigned_function`]).
fn js_member_path(
    left: &tree_sitter::Node,
    source: &str,
) -> Option<(String, String, &'static str)> {
    let mut member = Some(*left).filter(|l| l.kind() == "member_expression")?;
    let mut segments = Vec::new();
    let root_node;
    loop {
        let property = member.child_by_field_name("property")?;
        if property.kind() != "property_identifier" {
            return None;
        }
        segments.push(node_text(&property, source).to_string());
        let object = member.child_by_field_name("object")?;
        match object.kind() {
            "member_expression" => member = object,
            "identifier" => {
                segments.push(node_text(&object, source).to_string());
                root_node = object;
                break;
            }
            _ => return None,
        }
    }
    segments.reverse();
    let name = segments.last()?.clone();
    let is_prototype = segments[..segments.len() - 1]
        .iter()
        .any(|s| s == "prototype");
    // Only a function a module hands out: under `exports` / `module`, on a
    // prototype, or on an object that is no parameter or local of an enclosing
    // function and no host global. A test's mocks (`global.fetch = …`,
    // `req.end = () => {}`) are nobody's API, and as nodes they drew
    // production calls by name (pre-tag review 2026-09-29: 20 wrong
    // `inferred` edges on this repo, all into test stubs).
    let root = segments[0].as_str();
    if !(matches!(root, "exports" | "module")
        || is_prototype
        || relations::js_member_root_is_api(root_node, root, source))
    {
        return None;
    }
    let mut path: Vec<&str> = segments
        .iter()
        .enumerate()
        .filter(|(i, s)| *i == segments.len() - 1 || s.as_str() != "prototype")
        .map(|(_, s)| s.as_str())
        .collect();
    if path.len() > 2 && path[0] == "module" && path[1] == "exports" {
        path.remove(0);
    }
    let kind = if is_prototype { "method" } else { "function" };
    Some((name, path.join("."), kind))
}

/// The field name when `node` is the function value of a class field:
/// TS `json = (o) => {}` / `private readonly f = function () {}`
/// (`public_field_definition`) or JS `handler = () => {}` (`field_definition`).
/// hono's `Context` defines 15 of its methods this way. The caller qualifies it
/// with the enclosing class.
pub(crate) fn js_class_field_function(node: &tree_sitter::Node, source: &str) -> Option<String> {
    if !matches!(node.kind(), "arrow_function" | "function_expression") {
        return None;
    }
    let field = node
        .parent()
        .filter(|p| matches!(p.kind(), "public_field_definition" | "field_definition"))?;
    if field.child_by_field_name("value")?.id() != node.id() {
        return None;
    }
    let key = field
        .child_by_field_name("name")
        .or_else(|| field.child_by_field_name("property"))
        .filter(|k| k.kind() == "property_identifier")?;
    Some(node_text(&key, source).to_string())
}

#[cfg(test)]
mod tests {
    use super::rust_impl_type_name;

    #[test]
    fn rust_impl_type_name_drops_arguments_before_the_path() {
        for (text, want) in [
            ("Plain", "Plain"),
            ("Gen<T>", "Gen"),
            ("crate::db_a::Db", "Db"),
            ("crate::a::List<L, L::Target>", "List"),
            ("Outer<Inner<u8>, a::B>", "Outer"),
            ("<T as Link>::Target", "Target"),
            ("&'a mut Foo<T>", "&'a mut Foo"),
            // `->` inside the arguments is an arrow, not a closing `>` (pre-tag
            // review: `Box<dyn Fn() -> u8>` named its methods `Box u8>.run`).
            ("Box<dyn Fn() -> u8>", "Box"),
            ("std::boxed::Box<dyn Fn(u8) -> Vec<u8>>", "Box"),
        ] {
            assert_eq!(rust_impl_type_name(text), want, "{text}");
        }
    }
}
