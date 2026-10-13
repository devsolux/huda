//! Syntax-only constants retained from the supplied MIT-licensed parser.
//! These are extraction vocabulary, never relation authority or publication policy.

pub const REL_CALLS: &str = "calls";

pub const REL_INHERITS: &str = "inherits";

pub const REL_IMPORTS: &str = "imports";

pub const REL_ROUTES_TO: &str = "routes_to";

pub const REL_IMPLEMENTS: &str = "implements";

pub const REL_EXPORTS: &str = "exports";

pub const REL_REFERENCES: &str = "references";

pub const IMPORT_EXTERNAL_META: &str = r#"{"ext":1}"#;

pub const CALL_META_MEMBER: &str = r#"{"q":"member"}"#;

pub const CALL_Q_IMPORT: &str = "imp";

pub const CALL_Q_PACKAGE: &str = "pkg";

pub const CALL_KEY_UNTYPED_RECEIVER: &str = "ur";

pub const IMPORT_Q_NS_REQUIRE: &str = "ns_require";

pub const IMPORT_Q_NS_IMPORT: &str = "ns_import";

pub const IMPORT_Q_STAR_REEXPORT: &str = "star_reexport";

pub const IMPORT_Q_DEFAULT: &str = "default_import";

pub const MAX_AST_DEPTH: usize = 64;

pub const MAX_RELATION_DEPTH: usize = 256;

pub const INDEX_THREAD_STACK_SIZE: usize = 8 * 1024 * 1024;

pub const PYTHON_TYPE_REFERENCE_NOISE: &[&str] = &[
    // builtins
    "str",
    "int",
    "float",
    "bool",
    "bytes",
    "None",
    "object",
    "list",
    "dict",
    "set",
    "tuple",
    "frozenset",
    "complex",
    "type",
    // typing generics / special forms
    "Any",
    "List",
    "Dict",
    "Set",
    "Tuple",
    "FrozenSet",
    "Optional",
    "Union",
    "Callable",
    "Sequence",
    "Iterable",
    "Iterator",
    "Mapping",
    "MutableMapping",
    "Type",
    "ClassVar",
    "Final",
    "Literal",
    "Annotated",
    "NoReturn",
    "Self",
];

pub const GO_TYPE_REFERENCE_NOISE: &[&str] = &[
    "bool",
    "string",
    "error",
    "any",
    "rune",
    "byte",
    "uintptr",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "float32",
    "float64",
    "complex64",
    "complex128",
    "comparable",
];

pub const JAVA_TYPE_REFERENCE_NOISE: &[&str] = &[
    // java.lang (auto-imported)
    "String",
    "Object",
    "Integer",
    "Long",
    "Double",
    "Float",
    "Boolean",
    "Character",
    "Byte",
    "Short",
    "Number",
    "Void",
    "Class",
    "Exception",
    "RuntimeException",
    "Throwable",
    "Error",
    "Comparable",
    "Runnable",
    "Thread",
    "Iterable",
    // common annotations (java.lang / java.lang.annotation)
    "Override",
    "Deprecated",
    "SuppressWarnings",
    // common java.util collections + utilities
    "List",
    "ArrayList",
    "LinkedList",
    "Map",
    "HashMap",
    "TreeMap",
    "LinkedHashMap",
    "Set",
    "HashSet",
    "TreeSet",
    "Collection",
    "Optional",
    "Iterator",
    // java.util.stream
    "Stream",
];

pub fn is_external_import_meta(value: Option<&str>) -> bool {
    value == Some(IMPORT_EXTERNAL_META)
}
pub fn route_method_matches(stored: &str, requested: &str) -> bool {
    stored.eq_ignore_ascii_case("ANY")
        || stored.eq_ignore_ascii_case("ALL")
        || stored.eq_ignore_ascii_case(requested)
}
pub const fn max_code_content_len() -> usize {
    4096
}
pub const fn parse_timeout_ms() -> u64 {
    5000
}
