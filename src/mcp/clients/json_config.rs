//! A client's servers in a JSON file. The text is edited in place, so comments, order,
//! indentation and line endings around this one entry stay as the user wrote them.
use super::catalog::JsonShape;
use super::{invalid_config, split_bom, ServerEntry};
use crate::error::Result;
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use jsonc_parser::ParseOptions;
use serde_json::{json, Map, Value};

/// Comments and trailing commas are common in these files. The other loose forms are mistakes a
/// client would refuse, so they count as a broken file.
fn parse(text: &str) -> Result<CstRootNode> {
    let options = ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    };
    CstRootNode::parse(text, &options).map_err(|_| invalid_config())
}

fn entry_value(shape: JsonShape, entry: &ServerEntry) -> CstInputValue {
    let mut fields = Vec::new();
    if let Some(kind) = shape.kind {
        fields.push(("type".to_owned(), kind.into()));
    }
    fields.push(("command".to_owned(), entry.command.into()));
    let args = entry.args.iter().map(|arg| arg.as_str().into()).collect();
    fields.push(("args".to_owned(), CstInputValue::Array(args)));
    if shape.all_tools {
        fields.push(("tools".to_owned(), CstInputValue::Array(vec!["*".into()])));
    }
    CstInputValue::Object(fields)
}

/// The configuration with this server's entry set. A file that is not a JSON object with an
/// object under the servers key is refused, and never replaced.
pub fn with_server(current: &str, shape: JsonShape, entry: &ServerEntry) -> Result<String> {
    let (bom, text) = split_bom(current);
    let root = parse(text)?;
    // The `_or_set` forms would replace a value of another type, and the user's file with it.
    let servers = root
        .object_value_or_create()
        .and_then(|config| config.object_value_or_create(shape.servers))
        .ok_or_else(invalid_config)?;
    // A client lists names as written, so another spelling would show as a second server.
    for other in servers.properties() {
        let spelling = other.name().and_then(|name| name.decoded_value().ok());
        if spelling.is_some_and(|name| name != entry.name && name.eq_ignore_ascii_case(entry.name))
        {
            other.remove();
        }
    }
    match servers.get(entry.name) {
        Some(existing) => existing.set_value(entry_value(shape, entry)),
        None => {
            servers.append(entry.name, entry_value(shape, entry));
        }
    }
    Ok(format!("{bom}{root}"))
}

/// The configuration without this server's entry, or nothing when it has no such entry.
pub fn without_server(current: &str, shape: JsonShape, name: &str) -> Result<Option<String>> {
    let (bom, text) = split_bom(current);
    let root = parse(text)?;
    let entry = root
        .object_value()
        .and_then(|config| config.object_value(shape.servers))
        .and_then(|servers| servers.get(name));
    let Some(entry) = entry else {
        return Ok(None);
    };
    entry.remove();
    Ok(Some(format!("{bom}{root}")))
}

fn server_object(current: &str, shape: JsonShape, name: &str) -> Option<CstObject> {
    let (_, text) = split_bom(current);
    parse(text)
        .ok()?
        .object_value()?
        .object_value(shape.servers)?
        .object_value(name)
}

/// The command of this server's entry, read from the parsed file, so a comment never counts. A
/// disabled entry is not a connection.
pub fn server_command(current: &str, shape: JsonShape, name: &str) -> Option<String> {
    let server = server_object(current, shape, name)?;
    let disabled = server
        .get("disabled")
        .and_then(|flag| flag.value()?.as_boolean_lit())
        .is_some_and(|flag| flag.value());
    if disabled {
        return None;
    }
    server
        .get("command")?
        .value()?
        .as_string_lit()?
        .decoded_value()
        .ok()
}

/// The arguments of this server's entry. An entry without any has none.
pub fn server_args(current: &str, shape: JsonShape, name: &str) -> Vec<String> {
    server_object(current, shape, name)
        .and_then(|server| server.array_value("args"))
        .map(|args| {
            args.elements()
                .iter()
                .filter_map(|element| element.as_string_lit()?.decoded_value().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// What the user pastes into a client of this shape.
pub fn snippet(shape: JsonShape, entry: &ServerEntry) -> String {
    let mut server = Map::new();
    if let Some(kind) = shape.kind {
        server.insert("type".to_owned(), json!(kind));
    }
    server.insert("command".to_owned(), json!(entry.command));
    server.insert("args".to_owned(), json!(entry.args));
    if shape.all_tools {
        server.insert("tools".to_owned(), json!(["*"]));
    }
    let config = json!({ shape.servers: { entry.name: Value::Object(server) } });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::catalog::MCP_SERVERS;
    use super::*;

    const VS_CODE: JsonShape = JsonShape {
        servers: "servers",
        kind: Some("stdio"),
        all_tools: false,
    };
    const COPILOT: JsonShape = JsonShape {
        servers: "mcpServers",
        kind: Some("local"),
        all_tools: true,
    };

    fn entry(args: &[&str]) -> ServerEntry<'static> {
        ServerEntry {
            name: "Huda",
            command: "/opt/huda",
            args: args.iter().map(ToString::to_string).collect(),
        }
    }

    const COMMENTED: &str = r#"{
  // keep this comment
  "theme": "dark",
  "mcpServers": {
    /* the other server */
    "other": { "command": "other", "env": { "TOKEN": "keep-me" } },
  },
}
"#;

    #[test]
    fn adding_keeps_every_other_byte_and_refuses_nothing_valid() {
        let updated = with_server(COMMENTED, MCP_SERVERS, &entry(&["mcp", "stdio"])).unwrap();
        for kept in [
            "// keep this comment",
            "/* the other server */",
            "\"theme\": \"dark\"",
            "\"env\": { \"TOKEN\": \"keep-me\" }",
        ] {
            assert!(updated.contains(kept), "{kept} in {updated}");
        }
        assert_eq!(
            server_command(&updated, MCP_SERVERS, "Huda").as_deref(),
            Some("/opt/huda")
        );
        assert_eq!(server_args(&updated, MCP_SERVERS, "Huda"), ["mcp", "stdio"]);
    }

    #[test]
    fn removing_takes_only_this_entry_and_returns_the_rest_as_it_was() {
        let added = with_server(COMMENTED, MCP_SERVERS, &entry(&[])).unwrap();
        let removed = without_server(&added, MCP_SERVERS, "Huda")
            .unwrap()
            .unwrap();
        assert!(removed.contains("// keep this comment"));
        assert!(removed.contains("\"other\""));
        assert_eq!(server_command(&removed, MCP_SERVERS, "Huda"), None);
        assert_eq!(
            without_server(COMMENTED, MCP_SERVERS, "Huda").unwrap(),
            None
        );
    }

    #[test]
    fn a_second_write_replaces_the_entry_in_place() {
        let first = with_server("{}", MCP_SERVERS, &entry(&["old"])).unwrap();
        let second = with_server(&first, MCP_SERVERS, &entry(&["new"])).unwrap();
        assert_eq!(server_args(&second, MCP_SERVERS, "Huda"), ["new"]);
        assert_eq!(second.matches("\"Huda\"").count(), 1);
    }

    #[test]
    fn another_spelling_of_the_name_does_not_stay_as_a_second_server() {
        let current = r#"{ "mcpServers": { "huda": { "command": "old" } } }"#;
        let updated = with_server(current, MCP_SERVERS, &entry(&[])).unwrap();
        assert_eq!(server_command(&updated, MCP_SERVERS, "huda"), None);
        assert!(server_command(&updated, MCP_SERVERS, "Huda").is_some());
    }

    #[test]
    fn an_empty_file_becomes_a_valid_configuration() {
        let updated = with_server("", MCP_SERVERS, &entry(&[])).unwrap();
        assert!(serde_json::from_str::<Value>(&updated).is_ok(), "{updated}");
        assert!(server_command(&updated, MCP_SERVERS, "Huda").is_some());
    }

    #[test]
    fn a_broken_or_unexpected_file_is_refused() {
        for current in [
            "{ not json",
            "[]",
            "\"text\"",
            r#"{ "mcpServers": [] }"#,
            r#"{ "mcpServers": "x" }"#,
            r#"{ 'mcpServers': {} }"#,
        ] {
            let error = with_server(current, MCP_SERVERS, &entry(&[])).unwrap_err();
            assert_eq!(error.code, "CLIENT_CONFIG_INVALID", "{current}");
        }
    }

    #[test]
    fn a_byte_order_mark_and_windows_line_endings_stay() {
        let current = "\u{feff}{\r\n  \"a\": 1\r\n}\r\n";
        let updated = with_server(current, MCP_SERVERS, &entry(&[])).unwrap();
        assert!(updated.starts_with('\u{feff}'));
        assert!(server_command(&updated, MCP_SERVERS, "Huda").is_some());
        assert!(!updated.replace("\r\n", "").contains('\n'), "{updated:?}");
    }

    #[test]
    fn a_commented_out_entry_is_not_a_connection() {
        let current = r#"{ "mcpServers": { /* "Huda": { "command": "/opt/huda" } */ } }"#;
        assert_eq!(server_command(current, MCP_SERVERS, "Huda"), None);
    }

    #[test]
    fn a_disabled_entry_is_not_a_connection() {
        let current = r#"{ "mcpServers": { "Huda": { "command": "x", "disabled": true } } }"#;
        assert_eq!(server_command(current, MCP_SERVERS, "Huda"), None);
    }

    #[test]
    fn each_shape_writes_what_its_client_reads() {
        let vs_code = with_server("{}", VS_CODE, &entry(&[])).unwrap();
        let value: Value = serde_json::from_str(&vs_code).unwrap();
        assert_eq!(value["servers"]["Huda"]["type"], "stdio");
        let copilot = with_server("{}", COPILOT, &entry(&[])).unwrap();
        let value: Value = serde_json::from_str(&copilot).unwrap();
        assert_eq!(value["mcpServers"]["Huda"]["type"], "local");
        assert_eq!(value["mcpServers"]["Huda"]["tools"], json!(["*"]));
    }

    #[test]
    fn a_snippet_is_the_entry_a_write_would_make() {
        let snippet = snippet(COPILOT, &entry(&["mcp"]));
        let value: Value = serde_json::from_str(&snippet).unwrap();
        assert_eq!(value["mcpServers"]["Huda"]["command"], "/opt/huda");
        assert_eq!(value["mcpServers"]["Huda"]["args"], json!(["mcp"]));
    }
}
