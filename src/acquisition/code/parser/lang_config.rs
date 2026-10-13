/// Per-language configuration for AST parsing and relation extraction.
/// Centralizes language-specific flags that were previously scattered as
/// inline `if language == "X"` guards in treesitter.rs and relations.rs.
///
/// Note on call-expression node kinds: there is no `call_node_kind` field
/// here because the call-dispatch in `relations/mod.rs` cannot be driven
/// by a single string. Different languages have call-shaped nodes whose
/// children need genuinely different handling (Ruby `call` doubles as
/// `require`; PHP splits into `function_call_expression` /
/// `member_call_expression` / `scoped_call_expression`; Python's `call`
/// uses an `attribute` child for method names; etc). Adding a new
/// language with a non-standard call node kind requires a dedicated
/// match arm in `relations/mod.rs::extract_relations_from_tree`, not a
/// config tweak. See the per-language arms there for the pattern.
pub struct LanguageConfig {
    /// Language name (e.g., "rust", "typescript")
    pub name: &'static str,
    /// Whether this language has test attributes (e.g., Rust #[test])
    pub has_test_attributes: bool,
    /// AST node kind for method signatures (e.g., "method_signature" for Dart)
    pub method_signature_kind: Option<&'static str>,
    /// Whether to check prev_sibling for method definitions (Dart pattern)
    pub method_via_sibling: bool,
    /// Whether function_body nodes contain method definitions (Dart)
    pub function_body_has_methods: bool,
    /// Whether class context should be propagated for scope qualification
    pub has_class_context: bool,
    /// Interface naming convention detection (C# IFoo pattern)
    pub interface_by_prefix: bool,
}

impl LanguageConfig {
    pub fn for_language(language: &str) -> Self {
        match language {
            "rust" => Self {
                name: "rust",
                has_test_attributes: true,
                method_signature_kind: None,
                method_via_sibling: false,
                function_body_has_methods: false,
                has_class_context: false,
                interface_by_prefix: false,
            },
            "dart" => Self {
                name: "dart",
                has_test_attributes: false,
                method_signature_kind: Some("method_signature"),
                method_via_sibling: true,
                function_body_has_methods: true,
                has_class_context: true,
                interface_by_prefix: false,
            },
            "ruby" => Self {
                name: "ruby",
                has_test_attributes: false,
                method_signature_kind: None,
                method_via_sibling: false,
                function_body_has_methods: false,
                has_class_context: true,
                interface_by_prefix: false,
            },
            "csharp" => Self {
                name: "csharp",
                has_test_attributes: false,
                method_signature_kind: None,
                method_via_sibling: false,
                function_body_has_methods: false,
                has_class_context: true,
                interface_by_prefix: true,
            },
            "bash" => Self {
                name: "bash",
                has_test_attributes: false,
                method_signature_kind: None,
                method_via_sibling: false,
                function_body_has_methods: false,
                has_class_context: false,
                interface_by_prefix: false,
            },
            // Default config for most languages — enumerate known names for 'static lifetime,
            // fall through to "unknown" for unsupported languages.
            other => {
                let static_name = match other {
                    "typescript" => "typescript",
                    "tsx" => "tsx",
                    "javascript" => "javascript",
                    "jsx" => "jsx",
                    "go" => "go",
                    "python" => "python",
                    "java" => "java",
                    "c" => "c",
                    "cpp" => "cpp",
                    "kotlin" => "kotlin",
                    "php" => "php",
                    "swift" => "swift",
                    "html" => "html",
                    "css" => "css",
                    "markdown" => "markdown",
                    "json" => "json",
                    _ => "unknown",
                };
                Self {
                    name: static_name,
                    has_test_attributes: false,
                    method_signature_kind: None,
                    method_via_sibling: false,
                    function_body_has_methods: false,
                    has_class_context: true,
                    interface_by_prefix: false,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_config() {
        let config = LanguageConfig::for_language("rust");
        assert_eq!(config.name, "rust");
        assert!(config.has_test_attributes);
        assert!(config.method_signature_kind.is_none());
        assert!(!config.method_via_sibling);
        assert!(!config.function_body_has_methods);
        assert!(!config.has_class_context);
        assert!(!config.interface_by_prefix);
    }

    #[test]
    fn test_dart_config() {
        let config = LanguageConfig::for_language("dart");
        assert_eq!(config.name, "dart");
        assert!(!config.has_test_attributes);
        assert_eq!(config.method_signature_kind, Some("method_signature"));
        assert!(config.method_via_sibling);
        assert!(config.function_body_has_methods);
        assert!(config.has_class_context);
        assert!(!config.interface_by_prefix);
    }

    #[test]
    fn test_ruby_config() {
        let config = LanguageConfig::for_language("ruby");
        assert_eq!(config.name, "ruby");
        assert!(!config.has_test_attributes);
        assert!(config.method_signature_kind.is_none());
        assert!(!config.method_via_sibling);
        assert!(!config.function_body_has_methods);
        assert!(config.has_class_context);
        assert!(!config.interface_by_prefix);
    }

    #[test]
    fn test_csharp_config() {
        let config = LanguageConfig::for_language("csharp");
        assert_eq!(config.name, "csharp");
        assert!(!config.has_test_attributes);
        assert!(config.method_signature_kind.is_none());
        assert!(!config.method_via_sibling);
        assert!(!config.function_body_has_methods);
        assert!(config.has_class_context);
        assert!(config.interface_by_prefix);
    }

    #[test]
    fn test_bash_config() {
        let config = LanguageConfig::for_language("bash");
        assert_eq!(config.name, "bash");
        assert!(!config.has_test_attributes);
        assert!(config.method_signature_kind.is_none());
        assert!(!config.method_via_sibling);
        assert!(!config.function_body_has_methods);
        assert!(!config.has_class_context);
        assert!(!config.interface_by_prefix);
    }

    #[test]
    fn test_default_config_typescript() {
        let config = LanguageConfig::for_language("typescript");
        assert_eq!(config.name, "typescript");
        assert!(!config.has_test_attributes);
        assert!(config.method_signature_kind.is_none());
        assert!(!config.method_via_sibling);
        assert!(!config.function_body_has_methods);
        assert!(config.has_class_context);
        assert!(!config.interface_by_prefix);
    }

    #[test]
    fn test_default_config_unknown_language() {
        let config = LanguageConfig::for_language("haskell");
        assert_eq!(config.name, "unknown");
        assert!(!config.has_test_attributes);
    }

    /// P2-7 guard for feedback_lang_config_default_name: every detectable
    /// language (utils::config::SUPPORTED_LANGUAGES) must resolve a tree-sitter
    /// grammar AND a round-tripping lang_config name. A forgotten `for_language`
    /// arm falls to "unknown", which silently makes `config.name == "X"` guards
    /// in treesitter never fire — the language then extracts nothing. This turns
    /// that silent partial-add into a build failure.
    #[test]
    fn every_supported_language_has_consistent_config() {
        for &lang in crate::acquisition::code::SUPPORTED_LANGUAGES {
            assert!(
                crate::acquisition::code::parser::languages::get_language(lang).is_some(),
                "no tree-sitter grammar for supported language {lang:?} \
                 (parser::languages::get_language)",
            );
            let config = LanguageConfig::for_language(lang);
            assert_eq!(
                config.name, lang,
                "lang_config::for_language({lang:?}).name must round-trip, got {:?} — a \
                 forgotten static_name arm makes `config.name == {lang:?}` guards silently \
                 never fire (feedback_lang_config_default_name)",
                config.name,
            );
        }
    }
}
