//! The type of a Rust method call's receiver, where the source fixes it (D#112).
//!
//! `x.f()` names only `f`, and resolving it by name bound a project method of
//! another type: an atomic's `.load(Ordering)` bound `ProjectClassNames::load`,
//! a `String`'s `.as_str()` bound `Direction::as_str`, rusqlite's
//! `tx.commit()` bound `UncheckedSavepoint::commit`. When the source writes the
//! receiver's type down, the call carries it and the resolver
//! (`resolve::ProjectClassNames::rust_receiver_candidates`) narrows the
//! candidates:
//!
//! - `{"rt":"T","rk":"p"}`: a type of this crate. Only `T`'s own methods (its
//!   inherent and trait-impl methods, qualified `T.f`) are candidates when it
//!   has one; otherwise the call resolves as before (a trait's default method,
//!   a `Deref` target).
//! - `{"rt":"R","rk":"f"}`: a std/core/alloc type or a primitive (`R` may be
//!   empty: a slice, a tuple, an unsuffixed literal). A method of a project
//!   struct, enum or union other than one named `R` is no candidate; a trait's
//!   method is, and so is one of a blanket, reference, slice or primitive impl
//!   that can run on `R` (`resolve::foreign_receiver_owner`).
//! - `{"rt":"T","rc":"krate"}`: a type rooted at a crate name that is not
//!   std's; the resolver decides whether that crate is a package of the
//!   workspace (then as `"p"`) or a dependency (then as `"f"`).
//!
//! What gives a receiver a type (anything else leaves it untyped, resolved as
//! before):
//! - `self` in an `impl` block: the impl's type;
//! - a `let` with a type (`let x: T`) or whose value fixes one: `T::new(..)`,
//!   `T::default()`, `T::from(..)`, `T::with_capacity(..)`, `T { .. }`, a
//!   literal, `vec![..]`, `format!(..)`, `.to_string()`, `.clone()` of a typed
//!   value, a call of a free function of this file whose return type is
//!   written down, a zero-argument call of a std/foreign function (no
//!   turbofish), and `?` / `.unwrap()` / `.expect(..)` of those (a written
//!   `Result<T, _>` / `Option<T>` gives `T`); `Box`/`Rc`/`Arc::new(v)` is `v`'s;
//! - a function parameter `x: T` / `x: &T` / `x: &mut T` (and an annotated
//!   closure parameter);
//! - a `static` or `const` of the file;
//! - a field of a typed value whose struct this file defines (`self.buf`,
//!   `self.inner.flag`).
//!
//! Deliberately untyped: an unannotated closure parameter, a pattern binding
//! (`if let Some(x)`, `match`, `for`, destructuring `let`), a generic parameter
//! (`x: T` with `T: Trait`), `impl Trait`, `dyn Trait`, the return of any other
//! method or function, a tuple field (`self.0`), a field of a struct defined in
//! another file, and a foreign generic type that may deref to its argument
//! (`Arc<T>` gives `T`; `MutexGuard<T>`, `Cow<T>`, a dependency's `Guard<T>`
//! give nothing).

use std::cell::RefCell;
use std::collections::HashMap;

use super::helpers::MAX_SUBTREE_DEPTH;
use super::rust_use::{type_binding_name, type_origin, type_path, value_origin, TypeOrigin};
use super::{node_text, NamedChildAt};

/// A receiver's type as the resolver reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RecvTy {
    /// A project type: its name and, when a `use` or the written path names
    /// it, that path (normalized, `crate::loom::sync::Mutex`), which tells
    /// same-named types apart.
    Project(String, Option<String>),
    /// A project type behind a smart pointer (`Arc<T>`): `T`'s methods, or an
    /// impl on the pointer itself (`impl Schedule for Arc<Shared>`).
    Pointer(String, Option<String>, String),
    Foreign(String),
    /// A type rooted at a crate name that is not std's: name, crate, path.
    Crate(String, String, Option<String>),
}

/// The metadata keys for `ty` (see the module doc).
pub(super) fn receiver_keys(ty: &RecvTy) -> Vec<(&'static str, String)> {
    match ty {
        RecvTy::Project(n, path) => with_path(vec![("rt", n.clone()), ("rk", "p".into())], path),
        RecvTy::Pointer(n, path, via) => with_path(
            vec![("rt", n.clone()), ("rk", "p".into()), ("rv", via.clone())],
            path,
        ),
        RecvTy::Foreign(n) => vec![("rt", n.clone()), ("rk", "f".into())],
        RecvTy::Crate(n, c, path) => with_path(vec![("rt", n.clone()), ("rc", c.clone())], path),
    }
}

fn with_path(
    mut keys: Vec<(&'static str, String)>,
    path: &Option<String>,
) -> Vec<(&'static str, String)> {
    if let Some(p) = path {
        keys.push(("rp", p.clone()));
    }
    keys
}

/// Crates whose paths are std's.
const STD_ROOTS: &[&str] = &["std", "core", "alloc", "proc_macro"];

/// std names in the prelude or so common unimported that a type written with no
/// `use` and no local item of that name is std's.
const PRELUDE_TYPES: &[&str] = &["String", "Vec", "Option", "Result", "Box"];

/// std pointers whose methods are their argument's (`Deref`): typed as it.
const SMART_POINTERS: &[&str] = &["Box", "Rc", "Arc"];

/// std generic types with no `Deref` to their argument: their methods are
/// std's whatever the argument (a project trait's method still counts).
const NO_DEREF_GENERICS: &[&str] = &[
    "Vec",
    "VecDeque",
    "Option",
    "Result",
    "HashMap",
    "HashSet",
    "BTreeMap",
    "BTreeSet",
    "BinaryHeap",
    "LinkedList",
    "Mutex",
    "RwLock",
    "RefCell",
    "Cell",
    "OnceCell",
    "OnceLock",
    "Weak",
    "AtomicPtr",
    "Sender",
    "SyncSender",
    "Receiver",
    "JoinHandle",
    "PhantomData",
    "Range",
    "RangeInclusive",
];

/// Associated functions that return their type (`Self`) by convention or by
/// trait (`Default`, `From`).
const CONSTRUCTORS: &[&str] = &["new", "default", "from", "with_capacity"];

/// Traits a constructor may be called through, whose `Self` is not the path.
const CONSTRUCTOR_TRAITS: &[&str] = &["Default", "From", "Into", "FromIterator"];

const PRIMITIVES: &[&str] = &[
    "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64",
    "i128", "isize", "f32", "f64",
];

/// The receiver type of the Rust method call `call` (`x.f()`), or None.
pub(super) fn receiver_type(call: tree_sitter::Node, source: &str) -> Option<RecvTy> {
    let mut function = call.child_by_field_name("function")?;
    if function.kind() == "generic_function" {
        function = function.child_by_field_name("function")?;
    }
    if function.kind() != "field_expression" {
        return None;
    }
    let receiver = function.child_by_field_name("value")?;
    let mut cx = Cx { source, depth: 0 };
    let ty = cx.expr_type(receiver)?;
    cx.classify_ty(ty)
}

struct Cx<'s> {
    source: &'s str,
    depth: usize,
}

/// A value's type: a type node written in the source (classified where it is
/// written, where its `use` and generics apply), one already decided, or a
/// local function's return (see [`FnRet`]).
enum TyOut<'t> {
    Node(tree_sitter::Node<'t>),
    Recv(RecvTy),
    Fn(FnRet),
}

/// A free function's written return type, classified as it is and with a
/// `Result<T, _>` / `Option<T>` peeled (`f()?`, `f().unwrap()`).
#[derive(Clone, Default)]
struct FnRet {
    plain: Option<RecvTy>,
    peeled: Option<RecvTy>,
}

thread_local! {
    /// Per file (reset by [`reset`]): each free function's return type and
    /// each (struct, field)'s type, found by one scan of the file's items per
    /// name instead of one per call.
    static FN_RETURNS: RefCell<HashMap<String, Option<FnRet>>> = RefCell::new(HashMap::new());
    static FIELDS: RefCell<HashMap<(String, String), Option<FnRet>>> =
        RefCell::new(HashMap::new());
}

/// Forget the previous file's caches. MUST run once per file before its walk.
pub(super) fn reset() {
    FN_RETURNS.with(|c| c.borrow_mut().clear());
    FIELDS.with(|c| c.borrow_mut().clear());
}

impl<'s> Cx<'s> {
    fn text<'t>(&self, n: &tree_sitter::Node<'t>) -> &'s str {
        node_text(n, self.source)
    }

    fn classify_ty(&mut self, ty: TyOut) -> Option<RecvTy> {
        match ty {
            TyOut::Recv(r) => Some(r),
            TyOut::Node(n) => self.classify(n),
            TyOut::Fn(f) => f.plain,
        }
    }

    /// The type of expression `e`.
    fn expr_type<'t>(&mut self, e: tree_sitter::Node<'t>) -> Option<TyOut<'t>> {
        if self.depth > 8 {
            return None;
        }
        self.depth += 1;
        let out = self.expr_type_inner(e);
        self.depth -= 1;
        out
    }

    fn expr_type_inner<'t>(&mut self, e: tree_sitter::Node<'t>) -> Option<TyOut<'t>> {
        match e.kind() {
            "self" => self.self_type(e),
            "identifier" => self.binding_type(e, self.text(&e)),
            "field_expression" => self.field_type(e),
            "string_literal" | "raw_string_literal" => {
                Some(TyOut::Recv(RecvTy::Foreign("str".into())))
            }
            // `7u32` is a `u32`; an unsuffixed literal has no type yet.
            "integer_literal" | "float_literal" => Some(TyOut::Recv(RecvTy::Foreign(
                literal_suffix(self.text(&e))
                    .unwrap_or_default()
                    .to_string(),
            ))),
            "boolean_literal" => Some(TyOut::Recv(RecvTy::Foreign("bool".into()))),
            "char_literal" => Some(TyOut::Recv(RecvTy::Foreign("char".into()))),
            "reference_expression" => self.expr_type(e.child_by_field_name("value")?),
            "parenthesized_expression" => self.expr_type(e.named_child(0)?),
            "call_expression" | "try_expression" | "struct_expression" | "macro_invocation" => {
                self.value_type(e)
            }
            _ => None,
        }
    }

    /// The type a `let` value (or any expression) gives its binding.
    fn value_type<'t>(&mut self, v: tree_sitter::Node<'t>) -> Option<TyOut<'t>> {
        match v.kind() {
            "try_expression" => {
                let inner = self.value_type(v.named_child(0)?)?;
                self.peel(inner)
            }
            "call_expression" => self.call_type(v),
            "struct_expression" => Some(TyOut::Node(v.child_by_field_name("name")?)),
            "macro_invocation" => {
                let name = v.child_by_field_name("macro")?;
                match self.text(&name) {
                    "vec" => Some(TyOut::Recv(RecvTy::Foreign("Vec".into()))),
                    "format" => Some(TyOut::Recv(RecvTy::Foreign("String".into()))),
                    _ => None,
                }
            }
            _ => self.expr_type(v),
        }
    }

    /// `Result<T, _>` / `Option<T>` written down → `T`; anything else as it is
    /// (`T::new()?` is `T`).
    fn peel<'t>(&mut self, ty: TyOut<'t>) -> Option<TyOut<'t>> {
        let n = match ty {
            TyOut::Node(n) => n,
            TyOut::Fn(f) => return f.peeled.map(TyOut::Recv),
            // `Option`/`Result` decided without their argument: unknown inside.
            TyOut::Recv(RecvTy::Foreign(n)) if matches!(n.as_str(), "Option" | "Result") => {
                return None
            }
            TyOut::Recv(_) => return Some(ty),
        };
        let n = if n.kind() == "reference_type" {
            n.child_by_field_name("type")?
        } else {
            n
        };
        if n.kind() != "generic_type" {
            return Some(TyOut::Node(n));
        }
        let base = n.child_by_field_name("type")?;
        let last = self.text(&base).rsplit("::").next().unwrap_or_default();
        if !matches!(last, "Result" | "Option") {
            return Some(TyOut::Node(n));
        }
        first_type_argument(n).map(TyOut::Node)
    }

    fn call_type<'t>(&mut self, call: tree_sitter::Node<'t>) -> Option<TyOut<'t>> {
        let function = call.child_by_field_name("function")?;
        let args = call.child_by_field_name("arguments");
        let argc = args.map_or(0, |a| {
            (0..a.named_child_count())
                .filter_map(|i| a.named_child_at(i))
                .filter(|c| !c.is_extra() && c.kind() != "attribute_item")
                .count()
        });
        match function.kind() {
            "field_expression" => {
                let method = self.text(&function.child_by_field_name("field")?);
                let recv = function.child_by_field_name("value")?;
                match method {
                    "unwrap" | "expect" | "unwrap_or_default" => {
                        let inner = self.expr_type(recv)?;
                        self.peel(inner)
                    }
                    "to_string" => Some(TyOut::Recv(RecvTy::Foreign("String".into()))),
                    "clone" => self.expr_type(recv),
                    _ => None,
                }
            }
            "scoped_identifier" => {
                let segments = path_segments(function, self.source)?;
                let (f, owner) = segments.split_last()?;
                let owner_last = owner.last()?;
                if owner_last == "Self" && owner.len() == 1 {
                    if !CONSTRUCTORS.contains(&f.as_str()) {
                        return None;
                    }
                    return self.self_type(function);
                }
                let is_type = owner_last.starts_with(|c: char| c.is_ascii_uppercase())
                    || PRIMITIVES.contains(&owner_last.as_str());
                if CONSTRUCTORS.contains(&f.as_str()) && is_type {
                    if CONSTRUCTOR_TRAITS.contains(&owner_last.as_str()) {
                        return None;
                    }
                    if SMART_POINTERS.contains(&owner_last.as_str()) {
                        if f != "new" {
                            return None;
                        }
                        let arg = args?.named_child(0)?;
                        let inner = self.value_type(arg)?;
                        return match self.classify_ty(inner)? {
                            RecvTy::Project(p, path) | RecvTy::Pointer(p, path, _) => {
                                Some(TyOut::Recv(RecvTy::Pointer(p, path, owner_last.clone())))
                            }
                            other => Some(TyOut::Recv(other)),
                        };
                    }
                    let ty = self.classify_path(function, owner)?;
                    if let RecvTy::Foreign(name) = &ty {
                        if !NO_DEREF_GENERICS.contains(&name.as_str()) && may_deref(name) {
                            return None;
                        }
                    }
                    return Some(TyOut::Recv(ty));
                }
                // `Instant::now()`, `io::stdin()`: a std/foreign function with
                // no argument and no turbofish returns a std/foreign value.
                if argc == 0 {
                    return self.foreign_result(function, owner, is_type);
                }
                None
            }
            "identifier" => {
                let name = self.text(&function);
                match value_origin(&call, self.source, name) {
                    // A tuple struct's constructor, else a function of the file.
                    TypeOrigin::Project if name.starts_with(|c: char| c.is_ascii_uppercase()) => {
                        let real = self.real_name(call, &[name.to_string()]);
                        Some(TyOut::Recv(RecvTy::Project(real, None)))
                    }
                    TypeOrigin::Project => self.local_fn_return(call, name),
                    TypeOrigin::Extern(root) if argc == 0 => {
                        Some(TyOut::Recv(if STD_ROOTS.contains(&root.as_str()) {
                            RecvTy::Foreign(String::new())
                        } else {
                            RecvTy::Crate(String::new(), root, None)
                        }))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The value of a zero-argument call of `owner::f` when `owner` is std's
    /// or a dependency's: a value of `owner` when it is a type, else of a type
    /// we cannot name.
    fn foreign_result<'t>(
        &mut self,
        at: tree_sitter::Node<'t>,
        owner: &[String],
        is_type: bool,
    ) -> Option<TyOut<'t>> {
        if owner.len() == 1 && owner[0] == "mem" {
            return None; // `mem::zeroed()` is any type
        }
        let root = match type_origin(&at, self.source, owner) {
            TypeOrigin::Extern(root) => root,
            TypeOrigin::Unbound if is_type && PRELUDE_TYPES.contains(&owner[0].as_str()) => {
                "std".to_string()
            }
            _ => return None,
        };
        let name = if is_type {
            self.real_name(at, owner)
        } else {
            String::new()
        };
        if is_type && may_deref(&name) && !NO_DEREF_GENERICS.contains(&name.as_str()) {
            return None;
        }
        // Only std's `T::f()` is known to be a `T` (`Instant::now()`); another
        // crate's may be a workspace package's builder (`Thing::builder()`),
        // so it stays unnamed: untyped if that crate is the project's.
        Some(TyOut::Recv(if STD_ROOTS.contains(&root.as_str()) {
            RecvTy::Foreign(name)
        } else {
            RecvTy::Crate(String::new(), root, None)
        }))
    }

    /// The written return type of the module-level free function `name`, when
    /// every one of that name in the file agrees on it.
    fn local_fn_return<'t>(
        &mut self,
        call: tree_sitter::Node<'t>,
        name: &str,
    ) -> Option<TyOut<'t>> {
        if let Some(hit) = FN_RETURNS.with(|c| c.borrow().get(name).cloned()) {
            return hit.map(TyOut::Fn);
        }
        let mut found: Option<tree_sitter::Node<'t>> = None;
        let mut agreed = true;
        for_each_item(call, &mut |n| {
            if n.kind() == "function_item"
                && n.child_by_field_name("name")
                    .is_some_and(|x| node_text(&x, self.source) == name)
            {
                match (n.child_by_field_name("return_type"), found) {
                    (None, _) => agreed = false,
                    (Some(ret), Some(prev)) => {
                        if node_text(&prev, self.source) != node_text(&ret, self.source) {
                            agreed = false;
                        }
                    }
                    (Some(ret), None) => found = Some(ret),
                }
            }
        });
        let ret = found.filter(|_| agreed).map(|ret| self.both(ret));
        FN_RETURNS.with(|c| c.borrow_mut().insert(name.to_string(), ret.clone()));
        ret.map(TyOut::Fn)
    }

    /// A written type classified as it is and with `Result`/`Option` peeled.
    fn both(&mut self, ty: tree_sitter::Node) -> FnRet {
        let plain = self.classify(ty);
        let peeled = match self.peel(TyOut::Node(ty)) {
            Some(TyOut::Node(n)) => self.classify(n),
            _ => None,
        };
        FnRet { plain, peeled }
    }

    /// The type `self` has: the enclosing `impl` block's.
    fn self_type<'t>(&mut self, node: tree_sitter::Node<'t>) -> Option<TyOut<'t>> {
        let mut cur = node.parent();
        while let Some(n) = cur {
            match n.kind() {
                "impl_item" => return Some(TyOut::Node(n.child_by_field_name("type")?)),
                "trait_item" => return None,
                _ => cur = n.parent(),
            }
        }
        None
    }

    /// The type of the local, parameter, static or const `name` that `node`
    /// sees.
    fn binding_type<'t>(&mut self, node: tree_sitter::Node<'t>, name: &str) -> Option<TyOut<'t>> {
        let mut child = node;
        let mut cur = node.parent();
        while let Some(n) = cur {
            match n.kind() {
                "block" => {
                    let mut last: Option<tree_sitter::Node<'t>> = None;
                    for i in 0..n.named_child_count() {
                        let Some(stmt) = n.named_child_at(i) else {
                            continue;
                        };
                        if stmt.start_byte() >= child.start_byte() {
                            break;
                        }
                        let binds = match stmt.kind() {
                            "let_declaration" => stmt
                                .child_by_field_name("pattern")
                                .is_some_and(|p| pattern_mentions(p, name, self.source, 0)),
                            "static_item" | "const_item" => stmt
                                .child_by_field_name("name")
                                .is_some_and(|x| self.text(&x) == name),
                            _ => false,
                        };
                        if binds {
                            last = Some(stmt);
                        }
                    }
                    if let Some(decl) = last {
                        return self.declared(decl, name);
                    }
                }
                // The pattern binds in the loop body and the arm, not in the
                // iterated value or the scrutinee.
                "for_expression"
                    if n.child_by_field_name("body")
                        .is_some_and(|b| b.id() == child.id()) =>
                {
                    let pat = n.child_by_field_name("pattern");
                    if pat.is_some_and(|p| pattern_mentions(p, name, self.source, 0)) {
                        return None;
                    }
                }
                "match_arm" => {
                    let pat = n.child_by_field_name("pattern");
                    if pat.is_some_and(|p| pattern_mentions(p, name, self.source, 0)) {
                        return None;
                    }
                }
                "if_expression" | "while_expression" => {
                    let guarded = n
                        .child_by_field_name("consequence")
                        .or_else(|| n.child_by_field_name("body"))
                        .is_some_and(|b| b.id() == child.id());
                    if guarded {
                        if let Some(cond) = n.child_by_field_name("condition") {
                            if binds_in_let_conditions(cond, name, self.source, 0) {
                                return None;
                            }
                        }
                    }
                }
                "closure_expression" => {
                    if let Some(params) = n.child_by_field_name("parameters") {
                        for i in 0..params.named_child_count() {
                            let Some(p) = params.named_child_at(i) else {
                                continue;
                            };
                            let (pat, ty) = if p.kind() == "parameter" {
                                (
                                    p.child_by_field_name("pattern"),
                                    p.child_by_field_name("type"),
                                )
                            } else {
                                (Some(p), None)
                            };
                            if pat.is_some_and(|x| pattern_mentions(x, name, self.source, 0)) {
                                return (pat.is_some_and(|x| plain_binding(x, name, self.source)))
                                    .then_some(ty)
                                    .flatten()
                                    .map(TyOut::Node);
                            }
                        }
                    }
                }
                "function_item" => {
                    if let Some(params) = n.child_by_field_name("parameters") {
                        for i in 0..params.named_child_count() {
                            let Some(p) = params.named_child_at(i) else {
                                continue;
                            };
                            if p.kind() != "parameter" {
                                continue;
                            }
                            let Some(pat) = p.child_by_field_name("pattern") else {
                                continue;
                            };
                            if pattern_mentions(pat, name, self.source, 0) {
                                return plain_binding(pat, name, self.source)
                                    .then(|| p.child_by_field_name("type"))
                                    .flatten()
                                    .map(TyOut::Node);
                            }
                        }
                    }
                    // Not a local: a `static` / `const` of an enclosing module.
                    return self.module_item_type(n, name);
                }
                "source_file" | "declaration_list" => return self.module_item_type(child, name),
                _ => {}
            }
            child = n;
            cur = n.parent();
        }
        None
    }

    /// The type of `name` bound by `decl` (a `let`, `static` or `const`), when
    /// it binds `name` plainly.
    fn declared<'t>(&mut self, decl: tree_sitter::Node<'t>, name: &str) -> Option<TyOut<'t>> {
        if decl.kind() == "let_declaration" {
            let pat = decl.child_by_field_name("pattern")?;
            if !plain_binding(pat, name, self.source) {
                return None;
            }
        }
        if let Some(ty) = decl.child_by_field_name("type") {
            return Some(TyOut::Node(ty));
        }
        if decl.kind() != "let_declaration" {
            return None;
        }
        let value = decl.child_by_field_name("value")?;
        self.value_type(value)
    }

    /// A `static` / `const` named `name` among the items of the module holding
    /// `from`, else of the modules around it (a name the file's modules see).
    fn module_item_type<'t>(
        &mut self,
        from: tree_sitter::Node<'t>,
        name: &str,
    ) -> Option<TyOut<'t>> {
        let mut cur = Some(from);
        while let Some(n) = cur {
            if n.kind() == "source_file" || (n.kind() == "declaration_list" && is_mod_body(n)) {
                for i in 0..n.named_child_count() {
                    let Some(item) = n.named_child_at(i) else {
                        continue;
                    };
                    if matches!(item.kind(), "static_item" | "const_item")
                        && item
                            .child_by_field_name("name")
                            .is_some_and(|x| self.text(&x) == name)
                    {
                        return item.child_by_field_name("type").map(TyOut::Node);
                    }
                }
                // Modules do not inherit names (`use super::*` aside): stop.
                return None;
            }
            cur = n.parent();
        }
        None
    }

    /// `base.field`: the field's declared type, when `base` is typed as a
    /// struct this file defines (every struct of that name agreeing on it).
    fn field_type<'t>(&mut self, fe: tree_sitter::Node<'t>) -> Option<TyOut<'t>> {
        let field = fe.child_by_field_name("field")?;
        if field.kind() != "field_identifier" {
            return None;
        }
        let field = self.text(&field).to_string();
        let base = self.expr_type(fe.child_by_field_name("value")?)?;
        // A field through a smart pointer is its argument's (`Deref`).
        let (RecvTy::Project(owner, _) | RecvTy::Pointer(owner, _, _)) = self.classify_ty(base)?
        else {
            return None;
        };
        let key = (owner, field);
        if let Some(hit) = FIELDS.with(|c| c.borrow().get(&key).cloned()) {
            return hit.map(TyOut::Fn);
        }
        let mut found: Option<tree_sitter::Node<'t>> = None;
        let mut agreed = true;
        let (owner, field) = (&key.0, &key.1);
        for_each_item(fe, &mut |n| {
            let named = n.kind() == "struct_item"
                && n.child_by_field_name("name")
                    .is_some_and(|x| node_text(&x, self.source) == owner);
            if !named {
                return;
            }
            let ty = n
                .child_by_field_name("body")
                .filter(|b| b.kind() == "field_declaration_list")
                .and_then(|body| {
                    (0..body.named_child_count())
                        .filter_map(|i| body.named_child_at(i))
                        .filter(|d| d.kind() == "field_declaration")
                        .find(|d| {
                            d.child_by_field_name("name")
                                .is_some_and(|x| node_text(&x, self.source) == field)
                        })
                        .and_then(|d| d.child_by_field_name("type"))
                });
            match (ty, found) {
                (None, _) => agreed = false,
                (Some(t), Some(prev)) => {
                    if node_text(&prev, self.source) != node_text(&t, self.source) {
                        agreed = false;
                    }
                }
                (Some(t), None) => found = Some(t),
            }
        });
        let ty = found.filter(|_| agreed).map(|t| self.both(t));
        FIELDS.with(|c| c.borrow_mut().insert(key, ty.clone()));
        ty.map(TyOut::Fn)
    }

    /// What a type node names (see the module doc), or None.
    fn classify(&mut self, ty: tree_sitter::Node) -> Option<RecvTy> {
        if self.depth > 16 {
            return None;
        }
        self.depth += 1;
        let out = self.classify_inner(ty);
        self.depth -= 1;
        out
    }

    fn classify_inner(&mut self, ty: tree_sitter::Node) -> Option<RecvTy> {
        match ty.kind() {
            "reference_type" => self.classify(ty.child_by_field_name("type")?),
            "type_identifier" => {
                let name = self.text(&ty);
                if name == "Self" {
                    let TyOut::Node(n) = self.self_type(ty)? else {
                        return None;
                    };
                    return self.classify(n);
                }
                if is_generic_param(ty, name, self.source) {
                    return None;
                }
                self.classify_path(ty, &[name.to_string()])
            }
            "scoped_type_identifier" => {
                let segments = path_segments(ty, self.source)?;
                if segments.first().is_some_and(|s| s == "Self")
                    || is_generic_param(ty, &segments[0], self.source)
                {
                    return None; // an associated type
                }
                self.classify_path(ty, &segments)
            }
            "generic_type" | "generic_type_with_turbofish" => {
                let base = ty.child_by_field_name("type")?;
                let segments = path_segments(base, self.source)?;
                if segments.first().is_some_and(|s| s == "Self")
                    || is_generic_param(ty, &segments[0], self.source)
                {
                    return None;
                }
                let ty_arg = first_type_argument(ty);
                let last = segments.last()?;
                // A smart pointer, std's or a project's wrapper of it (tokio's
                // `loom::sync::Arc`): its argument's methods, or its own impls.
                if SMART_POINTERS.contains(&last.as_str()) {
                    return match self.classify(ty_arg?)? {
                        RecvTy::Project(inner, path) | RecvTy::Pointer(inner, path, _) => {
                            Some(RecvTy::Pointer(inner, path, last.clone()))
                        }
                        other => Some(other),
                    };
                }
                match self.classify_path(base, &segments)? {
                    // Lifetimes only (`Transaction<'_>`): no argument to deref to.
                    base if ty_arg.is_none() => Some(base),
                    RecvTy::Foreign(name) if NO_DEREF_GENERICS.contains(&name.as_str()) => {
                        Some(RecvTy::Foreign(name))
                    }
                    // A generic type may deref to its argument.
                    RecvTy::Foreign(_) | RecvTy::Crate(..) => None,
                    project => Some(project),
                }
            }
            "primitive_type" => Some(RecvTy::Foreign(self.text(&ty).to_string())),
            "array_type" | "tuple_type" | "unit_type" | "pointer_type" | "function_type"
            | "never_type" => Some(RecvTy::Foreign(String::new())),
            _ => None,
        }
    }

    /// The type a (type-namespace) path names, by where its first segment
    /// comes from.
    fn classify_path(&mut self, at: tree_sitter::Node, segments: &[String]) -> Option<RecvTy> {
        let last = segments.last()?;
        if segments.len() == 1 && PRIMITIVES.contains(&last.as_str()) {
            return Some(RecvTy::Foreign(last.clone()));
        }
        let name = self.real_name(at, segments);
        let path = || type_path(&at, self.source, segments).map(|p| p.join("::"));
        match type_origin(&at, self.source, segments) {
            TypeOrigin::Project => Some(RecvTy::Project(name, path())),
            TypeOrigin::Extern(root) if STD_ROOTS.contains(&root.as_str()) => {
                Some(RecvTy::Foreign(name))
            }
            TypeOrigin::Extern(root) => Some(RecvTy::Crate(name, root, path())),
            TypeOrigin::Unbound if PRELUDE_TYPES.contains(&last.as_str()) => {
                Some(RecvTy::Foreign(last.clone()))
            }
            TypeOrigin::Unbound => Some(RecvTy::Project(last.clone(), None)),
            TypeOrigin::Unknown => None,
        }
    }

    /// The name a path's type has: its last segment, or for a single name the
    /// type a `use … as` renames.
    fn real_name(&self, at: tree_sitter::Node, segments: &[String]) -> String {
        let last = segments.last().cloned().unwrap_or_default();
        if segments.len() == 1 {
            if let Some(real) = type_binding_name(&at, self.source, &last) {
                return real;
            }
        }
        last
    }
}

/// The type suffix of a numeric literal (`7u32`, `1.5f64`, `0xffu8`), or None.
/// A hex literal's trailing `f32` is digits (`0x1f32`), not a suffix.
fn literal_suffix(text: &str) -> Option<&'static str> {
    const SUFFIXES: &[&str] = &[
        "u128", "usize", "u16", "u32", "u64", "u8", "i128", "isize", "i16", "i32", "i64", "i8",
        "f32", "f64",
    ];
    let hex = text.starts_with("0x") || text.starts_with("0X");
    SUFFIXES
        .iter()
        .copied()
        .find(|s| text.ends_with(s) && !(hex && s.starts_with('f')))
}

/// Whether a foreign type may deref to a project type: a std smart pointer or
/// a guard/wrapper whose name says so.
fn may_deref(name: &str) -> bool {
    SMART_POINTERS.contains(&name)
        || matches!(
            name,
            "Pin" | "Cow" | "ManuallyDrop" | "AssertUnwindSafe" | "LazyLock" | "LazyCell"
        )
        || name.ends_with("Guard")
        || name.starts_with("Ref")
}

/// A path node's segments, generic arguments dropped: `a::B<T>::c` → a, B, c.
fn path_segments(node: tree_sitter::Node, source: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    collect_segments(node, source, &mut out, 0)?;
    (!out.is_empty()).then_some(out)
}

fn collect_segments(
    node: tree_sitter::Node,
    source: &str,
    out: &mut Vec<String>,
    depth: usize,
) -> Option<()> {
    if depth > MAX_SUBTREE_DEPTH {
        return None;
    }
    match node.kind() {
        "scoped_identifier" | "scoped_type_identifier" => {
            if let Some(path) = node.child_by_field_name("path") {
                collect_segments(path, source, out, depth + 1)?;
            }
            out.push(node_text(&node.child_by_field_name("name")?, source).to_string());
            Some(())
        }
        "identifier" | "type_identifier" | "crate" | "self" | "super" => {
            out.push(node_text(&node, source).to_string());
            Some(())
        }
        "generic_type" => {
            collect_segments(node.child_by_field_name("type")?, source, out, depth + 1)
        }
        _ => None,
    }
}

/// The first type argument of a generic type (`Arc<T>` → `T`), lifetimes skipped.
fn first_type_argument(ty: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let args = ty.child_by_field_name("type_arguments")?;
    (0..args.named_child_count())
        .filter_map(|i| args.named_child_at(i))
        .find(|a| !matches!(a.kind(), "lifetime" | "type_binding" | "trait_bounds"))
}

/// Whether `name` is a type parameter of an item around `node`.
fn is_generic_param(node: tree_sitter::Node, name: &str, source: &str) -> bool {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if let Some(params) = n.child_by_field_name("type_parameters") {
            let declares = (0..params.named_child_count())
                .filter_map(|i| params.named_child_at(i))
                .any(|p| {
                    let name_node = match p.kind() {
                        "type_parameter" => p.child_by_field_name("name"),
                        "type_identifier" => Some(p),
                        _ => p.named_child(0).filter(|c| c.kind() == "type_identifier"),
                    };
                    name_node.is_some_and(|x| node_text(&x, source) == name)
                });
            if declares {
                return true;
            }
        }
        cur = n.parent();
    }
    false
}

/// Call `f` on every item of the file holding `node`, modules included,
/// function bodies and `impl`/`trait` blocks not entered.
fn for_each_item<'t>(node: tree_sitter::Node<'t>, f: &mut dyn FnMut(tree_sitter::Node<'t>)) {
    let mut root = node;
    while let Some(p) = root.parent() {
        root = p;
    }
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        f(n);
        let container = match n.kind() {
            "source_file" => Some(n),
            "mod_item" => n.child_by_field_name("body"),
            // Items an item macro holds are no nodes: nothing to enter.
            _ => None,
        };
        if let Some(c) = container {
            for i in 0..c.named_child_count() {
                if let Some(item) = c.named_child_at(i) {
                    stack.push(item);
                }
            }
        }
    }
}

fn is_mod_body(n: tree_sitter::Node) -> bool {
    n.kind() == "declaration_list" && n.parent().is_some_and(|p| p.kind() == "mod_item")
}

/// Whether pattern `pat` mentions the identifier `name` (a binder, or a path in
/// it: over-counting only leaves a receiver untyped).
fn pattern_mentions(pat: tree_sitter::Node, name: &str, source: &str, depth: usize) -> bool {
    if depth > MAX_SUBTREE_DEPTH {
        return true;
    }
    if pat.kind() == "identifier" && node_text(&pat, source) == name {
        return true;
    }
    (0..pat.named_child_count())
        .filter_map(|i| pat.named_child_at(i))
        .any(|c| pattern_mentions(c, name, source, depth + 1))
}

/// Whether `pat` is `name` or `mut name` itself.
fn plain_binding(pat: tree_sitter::Node, name: &str, source: &str) -> bool {
    match pat.kind() {
        "identifier" => node_text(&pat, source) == name,
        "mut_pattern" => (0..pat.named_child_count())
            .filter_map(|i| pat.named_child_at(i))
            .any(|c| c.kind() == "identifier" && node_text(&c, source) == name),
        _ => false,
    }
}

/// Whether a condition (a `let` chain) holds a `let` binding `name`.
fn binds_in_let_conditions(
    cond: tree_sitter::Node,
    name: &str,
    source: &str,
    depth: usize,
) -> bool {
    if depth > MAX_SUBTREE_DEPTH {
        return true;
    }
    if cond.kind() == "let_condition" {
        return cond
            .child_by_field_name("pattern")
            .is_some_and(|p| pattern_mentions(p, name, source, 0));
    }
    (0..cond.named_child_count())
        .filter_map(|i| cond.named_child_at(i))
        .any(|c| binds_in_let_conditions(c, name, source, depth + 1))
}
