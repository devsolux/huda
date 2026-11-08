//! The real `huda` binary, connected to clients below a temporary home. It starts exactly what a
//! client's file holds and speaks to it as a client does. No client, Core, model or OS secret
//! store is involved: `server/discover` and refused handshakes need none of them.
#![cfg(target_os = "macos")]
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const HUDA: &str = env!("CARGO_BIN_EXE_huda");

struct Home(tempfile::TempDir);

impl Home {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.path().join(relative)
    }

    fn huda(&self, args: &[&str]) -> std::process::Output {
        Command::new(HUDA)
            .args(args)
            .env("HOME", self.0.path())
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.huda(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn row(&self, id: &str) -> Value {
        self.json(&["mcp", "clients", "list"])["clients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["client"] == id)
            .unwrap()
            .clone()
    }
}

/// Starts what `server` in a client's file holds, sends `frames`, and returns the answers. Standard
/// input stays open until each framed request is answered, as a client keeps it.
fn speak(home: &Home, server: &Value, frames: &[Value]) -> Vec<Value> {
    let args: Vec<&str> = server["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    let mut child = Command::new(server["command"].as_str().unwrap())
        .args(args)
        .env("HOME", home.0.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut answers = Vec::new();
    for frame in frames {
        writeln!(input, "{frame}").unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        answers.push(serde_json::from_str(&line).unwrap());
    }
    drop(input);
    child.wait().unwrap();
    answers
}

fn discover() -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": {"_meta": huda::mcp::request_meta()}})
}

fn initialize() -> Value {
    json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25",
        "capabilities": {},
        "clientInfo": {"name": "test", "version": "1"},
    }})
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_connected_client_starts_a_server_that_really_answers() {
    let home = Home::new();
    fs::create_dir_all(home.path(".cursor")).unwrap();
    home.json(&["mcp", "clients", "connect", "cursor", "--yes"]);
    assert_eq!(home.row("cursor")["state"], "configured");
    let file = read_json(&home.path(".cursor/mcp.json"));
    let answers = speak(&home, &file["mcpServers"]["Huda"], &[discover()]);
    assert_eq!(
        answers[0]["result"]["supportedVersions"],
        json!(["2026-07-28"])
    );
}

#[test]
fn nothing_changes_without_yes() {
    let home = Home::new();
    fs::create_dir_all(home.path(".cursor")).unwrap();
    let shown = home.json(&["mcp", "clients", "connect", "cursor"]);
    assert_eq!(shown["applied"], false);
    assert!(shown["preview"]["entry"]
        .as_str()
        .unwrap()
        .contains("--client"));
    assert!(!home.path(".cursor/mcp.json").exists());
    assert_eq!(home.row("cursor")["state"], "available");
}

#[test]
fn the_row_says_why_a_configured_client_failed() {
    let home = Home::new();
    fs::create_dir_all(home.path(".cursor")).unwrap();
    home.json(&["mcp", "clients", "connect", "cursor", "--yes"]);
    assert_eq!(home.row("cursor")["activity"]["refused_ms"], Value::Null);
    let file = read_json(&home.path(".cursor/mcp.json"));
    let answers = speak(&home, &file["mcpServers"]["Huda"], &[initialize()]);
    assert_eq!(answers[0]["error"]["code"], -32601);
    let row = home.row("cursor");
    assert_eq!(row["state"], "configured");
    assert!(row["activity"]["refused_ms"].is_u64(), "{row}");
}

#[test]
fn a_connection_without_a_client_name_serves_and_records_nothing() {
    let home = Home::new();
    let server = json!({"command": HUDA, "args": ["mcp", "stdio"]});
    let answers = speak(&home, &server, &[initialize()]);
    assert_eq!(answers[0]["error"]["code"], -32601);
    assert_eq!(home.row("cursor")["activity"]["refused_ms"], Value::Null);
    let unknown = json!({"command": HUDA, "args": ["mcp", "stdio", "--client", "../../escape"]});
    assert_eq!(
        speak(&home, &unknown, &[initialize()])[0]["error"]["code"],
        -32601
    );
}

#[test]
fn disconnecting_removes_only_this_entry() {
    let home = Home::new();
    fs::create_dir_all(home.path(".cursor")).unwrap();
    let original = "{\n  // mine\n  \"mcpServers\": { \"other\": { \"command\": \"other\" } }\n}\n";
    fs::write(home.path(".cursor/mcp.json"), original).unwrap();
    home.json(&["mcp", "clients", "connect", "cursor", "--yes"]);
    assert_eq!(
        fs::read_to_string(home.path(".cursor/mcp.json.huda-backup")).unwrap(),
        original
    );
    home.json(&["mcp", "clients", "disconnect", "cursor", "--yes"]);
    let after = fs::read_to_string(home.path(".cursor/mcp.json")).unwrap();
    assert!(
        after.contains("// mine") && after.contains("\"other\""),
        "{after}"
    );
    assert!(!after.contains("Huda"), "{after}");
    assert_eq!(home.row("cursor")["state"], "available");
}

#[test]
fn another_profile_stays_in_the_entry_and_the_default_row_needs_repair() {
    let home = Home::new();
    fs::create_dir_all(home.path(".cursor")).unwrap();
    let profile = home.path("work-profile");
    fs::create_dir_all(&profile).unwrap();
    let profile = profile.to_str().unwrap();
    let mut permissions = fs::metadata(profile).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o700);
    fs::set_permissions(profile, permissions).unwrap();
    home.json(&[
        "--profile",
        profile,
        "mcp",
        "clients",
        "connect",
        "cursor",
        "--yes",
    ]);
    let file = read_json(&home.path(".cursor/mcp.json"));
    let args = file["mcpServers"]["Huda"]["args"].as_array().unwrap();
    assert_eq!(args[0], "--profile");
    assert_eq!(home.row("cursor")["state"], "needs_repair");
}

#[test]
fn a_hand_connected_client_is_refused_a_change_and_an_unknown_one_is_named() {
    let home = Home::new();
    let shown = home.json(&["mcp", "clients", "connect", "goose", "--yes"]);
    assert_eq!(shown["applied"], false);
    assert!(shown["preview"]["entry"].as_str().unwrap().contains("Huda"));
    let unknown = home.huda(&["mcp", "clients", "connect", "nope", "--yes"]);
    assert!(!unknown.status.success());
    let not_here = home.huda(&["mcp", "clients", "connect", "kiro", "--yes"]);
    assert!(String::from_utf8_lossy(&not_here.stderr).contains("CLIENT_NOT_INSTALLED"));
    assert!(!home.path(".kiro").exists());
}

#[test]
fn listing_starts_no_core_and_opens_no_database() {
    let home = Home::new();
    let listed = home.json(&["mcp", "clients", "list"]);
    assert_eq!(listed["clients"].as_array().unwrap().len(), 20);
    assert_eq!(listed["profile_initialized"], false);
    let profile = home.path(".devsolux/huda");
    assert_eq!(fs::read_dir(profile).unwrap().count(), 0);
}

#[test]
fn a_new_home_gets_its_profile_in_the_devsolux_folder() {
    let home = Home::new();
    home.json(&["mcp", "clients", "list"]);
    assert!(home.path(".devsolux/huda").is_dir());
    assert!(!home.path("Library/Application Support/Huda").exists());
}

#[test]
fn a_profile_from_an_earlier_release_is_used_where_it_is() {
    let home = Home::new();
    let legacy = home.path("Library/Application Support/Huda");
    fs::create_dir_all(&legacy).unwrap();
    let mut permissions = fs::metadata(&legacy).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o700);
    fs::set_permissions(&legacy, permissions).unwrap();
    fs::write(legacy.join("huda.db"), b"").unwrap();
    let listed = home.json(&["mcp", "clients", "list"]);
    assert_eq!(listed["profile_initialized"], true);
    assert!(
        !home.path(".devsolux").exists(),
        "nothing new may be created"
    );
}
