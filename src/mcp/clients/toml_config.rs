//! The `[mcp_servers.<name>]` table of a TOML configuration. The document is edited in place, so
//! comments and order around this one table stay as the user wrote them.
use super::{invalid_config, split_bom, ServerEntry};
use crate::error::Result;
use toml_edit::{value, Array, DocumentMut, Item, Table};

const SERVERS: &str = "mcp_servers";

fn parse(text: &str) -> Result<DocumentMut> {
    text.parse().map_err(|_| invalid_config())
}

fn fill(table: &mut Table, entry: &ServerEntry) {
    table.insert("command", value(entry.command));
    table.insert(
        "args",
        value(Array::from_iter(entry.args.iter().map(String::as_str))),
    );
}

fn line_ending(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// The editor writes `\n`, so a Windows file gets its own line ending back.
fn with_line_ending(text: &str, line_ending: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', line_ending)
}

/// The configuration with this server's table set. An existing table is emptied and filled, so
/// it keeps its place and the comment above it, and no subtable can be left behind.
pub fn with_server(current: &str, entry: &ServerEntry) -> Result<String> {
    let (bom, text) = split_bom(current);
    let ending = line_ending(text);
    let mut document = parse(text)?;
    let is_empty = document.as_table().is_empty();
    let servers = document
        .as_table_mut()
        .entry(SERVERS)
        .or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(true);
            Item::Table(table)
        })
        .as_table_like_mut()
        .ok_or_else(invalid_config)?;
    // A client lists names as written, so another spelling would show as a second server.
    let spellings: Vec<String> = servers
        .iter()
        .map(|(name, _)| name.to_owned())
        .filter(|name| name != entry.name && name.eq_ignore_ascii_case(entry.name))
        .collect();
    for spelling in spellings {
        servers.remove(&spelling);
    }
    if let Some(table) = servers.get_mut(entry.name).and_then(Item::as_table_mut) {
        table.clear();
        fill(table, entry);
    } else {
        let mut table = Table::new();
        fill(&mut table, entry);
        if !is_empty {
            table.decor_mut().set_prefix("\n");
        }
        servers.insert(entry.name, Item::Table(table));
    }
    Ok(format!(
        "{bom}{}",
        with_line_ending(&document.to_string(), ending)
    ))
}

/// The configuration without this server's table, or nothing when it has no such table.
pub fn without_server(current: &str, name: &str) -> Result<Option<String>> {
    let (bom, text) = split_bom(current);
    let ending = line_ending(text);
    let mut document = parse(text)?;
    let removed = document
        .get_mut(SERVERS)
        .and_then(Item::as_table_like_mut)
        .and_then(|servers| servers.remove(name));
    Ok(removed.map(|_| format!("{bom}{}", with_line_ending(&document.to_string(), ending))))
}

/// The command of this server's table. A table with `enabled = false` is not a connection, and a
/// comment never counts because this reads the parsed document.
pub fn server_command(current: &str, name: &str) -> Option<String> {
    let (_, text) = split_bom(current);
    let document = parse(text).ok()?;
    let entry = document.get(SERVERS)?.get(name)?;
    if entry.get("enabled").and_then(Item::as_bool) == Some(false) {
        return None;
    }
    Some(entry.get("command")?.as_str()?.to_owned())
}

/// The arguments of this server's table. A table without any has none.
pub fn server_args(current: &str, name: &str) -> Vec<String> {
    let args = parse(split_bom(current).1).ok().and_then(|document| {
        let args = document.get(SERVERS)?.get(name)?.get("args")?.as_array()?;
        Some(
            args.iter()
                .filter_map(|arg| arg.as_str().map(str::to_owned))
                .collect(),
        )
    });
    args.unwrap_or_default()
}

/// What the user pastes into `config.toml`.
pub fn snippet(entry: &ServerEntry) -> String {
    let mut table = Table::new();
    fill(&mut table, entry);
    let mut servers = Table::new();
    servers.set_implicit(true);
    servers.insert(entry.name, Item::Table(table));
    let mut document = DocumentMut::new();
    document.insert(SERVERS, Item::Table(servers));
    document.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(args: &[&str]) -> ServerEntry<'static> {
        ServerEntry {
            name: "Huda",
            command: "/opt/huda",
            args: args.iter().map(ToString::to_string).collect(),
        }
    }

    const COMMENTED: &str = "# my settings\nmodel = \"o4\" # inline\n\n# their server\n[mcp_servers.other]\ncommand = \"other\"\nenv = { TOKEN = \"keep-me\" }\n";

    #[test]
    fn adding_keeps_comments_order_and_other_servers() {
        let updated = with_server(COMMENTED, &entry(&["mcp", "stdio"])).unwrap();
        assert!(updated.starts_with("# my settings\nmodel = \"o4\" # inline\n"));
        assert!(updated.contains("# their server\n[mcp_servers.other]\ncommand = \"other\"\n"));
        assert!(updated.contains("TOKEN = \"keep-me\""));
        assert_eq!(
            server_command(&updated, "Huda").as_deref(),
            Some("/opt/huda")
        );
        assert_eq!(server_args(&updated, "Huda"), ["mcp", "stdio"]);
    }

    #[test]
    fn removing_returns_the_rest_as_it_was() {
        let added = with_server(COMMENTED, &entry(&[])).unwrap();
        let removed = without_server(&added, "Huda").unwrap().unwrap();
        assert!(removed.contains("# their server"));
        assert_eq!(server_command(&removed, "Huda"), None);
        assert_eq!(without_server(COMMENTED, "Huda").unwrap(), None);
    }

    #[test]
    fn a_second_write_leaves_no_old_keys_behind() {
        let first = with_server("[mcp_servers.Huda]\nstale = 1\n", &entry(&["a"])).unwrap();
        assert!(!first.contains("stale"));
        let second = with_server(&first, &entry(&["b"])).unwrap();
        assert_eq!(server_args(&second, "Huda"), ["b"]);
    }

    #[test]
    fn another_spelling_of_the_name_does_not_stay_as_a_second_server() {
        let updated = with_server("[mcp_servers.HUDA]\ncommand = \"x\"\n", &entry(&[])).unwrap();
        assert_eq!(server_command(&updated, "HUDA"), None);
        assert!(server_command(&updated, "Huda").is_some());
    }

    #[test]
    fn a_broken_file_or_a_non_table_is_refused() {
        for current in ["not = = toml", "mcp_servers = 3\n"] {
            let error = with_server(current, &entry(&[])).unwrap_err();
            assert_eq!(error.code, "CLIENT_CONFIG_INVALID", "{current}");
        }
    }

    #[test]
    fn a_commented_out_or_disabled_table_is_not_a_connection() {
        assert_eq!(
            server_command("# [mcp_servers.Huda]\n# command = \"x\"\n", "Huda"),
            None
        );
        assert_eq!(
            server_command(
                "[mcp_servers.Huda]\ncommand = \"x\"\nenabled = false\n",
                "Huda"
            ),
            None
        );
    }

    #[test]
    fn a_byte_order_mark_and_windows_line_endings_stay() {
        let current = "\u{feff}model = \"o4\"\r\n";
        let updated = with_server(current, &entry(&[])).unwrap();
        assert!(updated.starts_with('\u{feff}'));
        assert!(!updated.replace("\r\n", "").contains('\n'), "{updated:?}");
    }

    #[test]
    fn a_snippet_can_be_read_back() {
        let snippet = snippet(&entry(&["mcp"]));
        assert_eq!(
            server_command(&snippet, "Huda").as_deref(),
            Some("/opt/huda")
        );
        assert_eq!(server_args(&snippet, "Huda"), ["mcp"]);
    }
}
