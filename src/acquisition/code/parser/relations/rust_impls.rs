//! Which methods of a file's OTHER impl blocks a `self.m()` / `Self::m()` call
//! can never run (D#159).
//!
//! Impls of one type that differ only in their arguments
//! (`impl AsyncWrite for Cursor<&mut [u8]>`, `… for Cursor<Vec<u8>>`) all name
//! their type `Cursor` (`rust_impl_type_name`), so by type name a `self.m()` in
//! one reaches every one of them. Two facts of the language tell them apart
//! without knowing what the arguments mean:
//!
//! - Coherence: two impls of the SAME trait never cover one type. A
//!   `self.m()` in `impl W for A` runs `A`'s `W::m`, so the `m` of another
//!   `impl W for …` block is another type's. This holds through aliases too,
//!   since impls of one trait for alias-equal types would conflict.
//! - E0592: two inherent impls of one type may not both define `m`. When the
//!   caller's own inherent impl defines `m`, another inherent impl's `m` is
//!   another type's. When it does not, the other block may be the same type's
//!   split impl, so nothing is excluded.
//!
//! Impls of different traits, and impls in other files, stay candidates: the
//! rule only removes what the language rules out. The result is recorded on the
//! call as the lines of the excluded methods (`"xl"`), read back by
//! `resolve::self_filter_candidates` for candidates in the caller's own file,
//! which a re-index always re-parses together with the call.

use super::node_text;
use std::cell::RefCell;
use std::collections::HashMap;

struct ImplBlock {
    /// `rust_impl_type_name` of the impl's type.
    ty: String,
    /// The implemented trait as written, whitespace dropped (`From<u8>` and
    /// `From<u16>` are different traits); None for an inherent impl.
    of_trait: Option<String>,
    /// Methods defined directly in the block: name, 1-based start line (the
    /// line their node is stored with), and whether a `#[cfg(…)]` gates it.
    methods: Vec<(String, u32, bool)>,
    /// The innermost inline `mod` holding the block (its node id), None at the
    /// file's top level.
    module: Option<usize>,
}

impl ImplBlock {
    fn defines(&self, method: &str) -> bool {
        self.methods.iter().any(|(m, _, _)| m == method)
    }

    /// Defines `method` wherever a caller in this block exists: that
    /// definition carries no `#[cfg(…)]` of its own. One on the whole block
    /// gates the caller with it, so it does not count.
    fn always_defines(&self, method: &str) -> bool {
        self.methods.iter().any(|(m, _, cfg)| m == method && !cfg)
    }
}

/// Whether a `#[cfg(…)]` attribute sits directly before `item`.
fn cfg_gated(item: tree_sitter::Node, source: &str) -> bool {
    let mut prev = item.prev_named_sibling();
    while let Some(p) = prev {
        if p.kind() != "attribute_item" {
            break;
        }
        let text: String = node_text(&p, source)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if text.starts_with("#[cfg(") {
            return true;
        }
        prev = p.prev_named_sibling();
    }
    false
}

/// The innermost inline `mod { … }` holding `node`, None at top level.
fn enclosing_module(node: tree_sitter::Node) -> Option<usize> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == "mod_item" {
            return Some(n.id());
        }
        cur = n.parent();
    }
    None
}

thread_local! {
    /// The current file's impl blocks, keyed by the `impl_item` node id. Built on
    /// first use per file and cleared by [`reset`]: node ids are unique only
    /// within one tree.
    static BLOCKS: RefCell<Option<HashMap<usize, ImplBlock>>> = const { RefCell::new(None) };
}

/// Drop the current file's blocks. Called once per file before its walk.
pub(super) fn reset() {
    BLOCKS.with(|b| *b.borrow_mut() = None);
}

fn build(root: tree_sitter::Node, source: &str) -> HashMap<usize, ImplBlock> {
    let mut blocks = HashMap::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "impl_item" {
            if let Some(ty) = node.child_by_field_name("type") {
                let of_trait = node.child_by_field_name("trait").map(|t| {
                    node_text(&t, source)
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect()
                });
                let mut methods = Vec::new();
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cursor = body.walk();
                    for item in body.named_children(&mut cursor) {
                        if item.kind() != "function_item" {
                            continue;
                        }
                        if let Some(name) = item.child_by_field_name("name") {
                            methods.push((
                                node_text(&name, source).to_string(),
                                item.start_position().row as u32 + 1,
                                cfg_gated(item, source),
                            ));
                        }
                    }
                }
                blocks.insert(
                    node.id(),
                    ImplBlock {
                        ty: crate::acquisition::code::parser::rust_impl_type_name(node_text(
                            &ty, source,
                        )),
                        of_trait,
                        methods,
                        module: enclosing_module(node),
                    },
                );
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    blocks
}

/// Whether the impl block around `call` is an inherent one (`impl Foo`, no
/// trait). Such an impl lives in its type's crate (E0116), and so does every
/// impl that crate can call on the type: another crate implementing a trait for
/// it would have to depend on this one. A `self`/`Self` call there never
/// reaches outside the crate.
pub(super) fn in_inherent_impl(call: tree_sitter::Node) -> bool {
    let mut cur = call.parent();
    while let Some(n) = cur {
        match n.kind() {
            "impl_item" => return n.child_by_field_name("trait").is_none(),
            "trait_item" => return false,
            _ => cur = n.parent(),
        }
    }
    false
}

/// What a `self`/`Self` call's own file says about its target.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct SelfCallFacts {
    /// Start lines of the file's methods of that name the call cannot reach:
    /// other impls the language keeps apart from the caller's (module doc).
    pub excluded: Vec<u32>,
    /// The file's own methods of that name are no proof of the target, and
    /// the crate decides (`resolve::self_filter_candidates`, `"wide"`): the
    /// file defines them only in trait impls, which an inherent method of that
    /// name in another file outranks; or some sit in another inline `mod`,
    /// where the type's name may be another type's.
    pub trait_only_here: bool,
}

/// [`SelfCallFacts`] for a `self`/`Self` call at `call` of `method`. Empty
/// outside an impl block.
pub(super) fn self_call_facts(
    call: tree_sitter::Node,
    source: &str,
    method: &str,
) -> SelfCallFacts {
    let mut cur = call.parent();
    let mut caller_line = None;
    let own = loop {
        match cur {
            Some(n) if n.kind() == "impl_item" => break n,
            Some(n) if n.kind() == "trait_item" => return SelfCallFacts::default(),
            Some(n) => {
                if n.kind() == "function_item" && caller_line.is_none() {
                    caller_line = Some(n.start_position().row as u32 + 1);
                }
                cur = n.parent();
            }
            None => return SelfCallFacts::default(),
        }
    };
    BLOCKS.with(|cell| {
        if cell.borrow().is_none() {
            let mut root = call;
            while let Some(p) = root.parent() {
                root = p;
            }
            *cell.borrow_mut() = Some(build(root, source));
        }
        let blocks = cell.borrow();
        let blocks = blocks.as_ref().expect("built above");
        let Some(mine) = blocks.get(&own.id()) else {
            return SelfCallFacts::default();
        };
        // E0592 keeps another inherent `m` apart only when the caller's own
        // is there in every build the call is: a `#[cfg]` on that `m` may
        // leave the other one the only `m` (pre-tag review).
        let apart = mine.of_trait.is_some() || mine.always_defines(method);
        // The caller's own inherent impl defining `m` is the target (E0592),
        // whatever else the file holds.
        let own_decides = mine.of_trait.is_none() && mine.always_defines(method);
        let mut excluded = Vec::new();
        let (mut inherent_here, mut trait_here) = (false, false);
        // Blocks of the type's name in another inline `mod` may be another
        // type of that name (a test module's mock): the file then holds two
        // types of one name, and its own methods are no proof of the target.
        let mut other_module = false;
        for (id, other) in blocks.iter().filter(|(_, b)| b.ty == mine.ty) {
            if !own_decides && other.module != mine.module && other.defines(method) {
                other_module = true;
            }
            for (_, line, _) in other.methods.iter().filter(|(m, _, _)| m == method) {
                if Some(*line) == caller_line && *id == own.id() {
                    continue; // the caller itself says nothing about its callee
                }
                if apart && *id != own.id() && other.of_trait == mine.of_trait {
                    excluded.push(*line);
                } else if other.of_trait.is_none() {
                    inherent_here = true;
                } else {
                    trait_here = true;
                }
            }
        }
        excluded.sort_unstable();
        excluded.dedup();
        SelfCallFacts {
            excluded,
            trait_only_here: (trait_here && !inherent_here) || other_module,
        }
    })
}

/// Whether `Pin<Ptr>` itself has `method`, found before `Self` through
/// `Deref`: every `Pin` has `as_ref`; one over a mutable pointer (`&mut T`,
/// `Box<T>`) also `as_mut` and `set`; `Pin<&T>` has `get_ref` and
/// `map_unchecked`, `Pin<&mut T>` `get_mut`, `get_unchecked_mut`, `into_ref`
/// and `map_unchecked_mut`. A method its pointer lacks is `Self`'s
/// (`*self.get_ref()` in a `Pin<&mut Self>` poll calls `Self::get_ref`). An
/// unrecognized pointer answers `as_ref` only, which leaves every other call
/// its edge to `Self`.
fn pin_has(pointer: Option<tree_sitter::Node>, source: &str, method: &str) -> bool {
    #[derive(PartialEq)]
    enum Ptr {
        Shared,
        Unique,
        Boxed,
        Other,
    }
    let ptr = match pointer {
        Some(p) if p.kind() == "reference_type" => {
            let mut c = p.walk();
            let unique = p
                .children(&mut c)
                .any(|ch| ch.kind() == "mutable_specifier");
            if unique {
                Ptr::Unique
            } else {
                Ptr::Shared
            }
        }
        Some(p) if p.kind() == "generic_type" => {
            let base = p
                .child_by_field_name("type")
                .map(|b| node_text(&b, source))
                .unwrap_or("");
            if base.rsplit("::").next() == Some("Box") {
                Ptr::Boxed
            } else {
                Ptr::Other
            }
        }
        _ => Ptr::Other,
    };
    match method {
        "as_ref" => true,
        "as_mut" | "set" => ptr == Ptr::Unique || ptr == Ptr::Boxed,
        "get_ref" | "map_unchecked" => ptr == Ptr::Shared,
        "get_mut" | "get_unchecked_mut" | "into_ref" | "map_unchecked_mut" => ptr == Ptr::Unique,
        _ => false,
    }
}

/// Whether a `self.method()` call is answered by the wrapper its enclosing
/// function declares `self` as, not by `Self`: with `self: Pin<&mut Self>`,
/// `self.get_mut()` is `Pin::get_mut` ([`pin_has`]), and with `self: Arc<Self>`,
/// `self.clone()` is `Arc`'s — method lookup meets the receiver's own type
/// before any deref reaches `Self`. The wrapper is read by name, as the parser
/// reads these pointers everywhere (`rust_receiver.rs`).
pub(super) fn answered_by_the_self_wrapper(
    call: tree_sitter::Node,
    source: &str,
    method: &str,
) -> bool {
    let mut cur = call.parent();
    let function = loop {
        match cur {
            Some(n) if n.kind() == "function_item" => break n,
            Some(n) if matches!(n.kind(), "impl_item" | "trait_item" | "source_file") => {
                return false
            }
            Some(n) => cur = n.parent(),
            None => return false,
        }
    };
    let Some(params) = function.child_by_field_name("parameters") else {
        return false;
    };
    let mut cursor = params.walk();
    let Some(receiver) = params
        .named_children(&mut cursor)
        .find(|p| p.kind() != "attribute_item")
    else {
        return false;
    };
    if receiver.kind() != "parameter"
        || receiver
            .child_by_field_name("pattern")
            .is_none_or(|p| p.kind() != "self")
    {
        return false;
    }
    let Some(ty) = receiver.child_by_field_name("type") else {
        return false;
    };
    if ty.kind() != "generic_type" {
        return false;
    }
    let Some(base) = ty.child_by_field_name("type") else {
        return false;
    };
    let text = node_text(&base, source);
    match text.rsplit("::").next().unwrap_or(text) {
        "Pin" => {
            let pointer = ty.child_by_field_name("type_arguments").and_then(|args| {
                let mut c = args.walk();
                let first = args.named_children(&mut c).next();
                first
            });
            pin_has(pointer, source, method)
        }
        "Arc" | "Rc" => method == "clone",
        _ => false,
    }
}
