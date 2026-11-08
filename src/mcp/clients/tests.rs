//! The rows and changes of real catalog entries, on folders below a temporary home.
use super::catalog::{self, Place, CATALOG};
use super::*;
use std::fs;

struct Scratch {
    /// Keeps the folder alive for every test; only the Unix-only `own_command` tests read it.
    #[cfg_attr(not(unix), allow(dead_code))]
    home: tempfile::TempDir,
    folders: Folders,
    server: Server,
}

impl Scratch {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let folders = Folders::below(home.path());
        Self {
            home,
            folders,
            server: server("/opt/huda/huda", None),
        }
    }

    fn entry(client: ClientId) -> &'static Entry {
        catalog::entry(client).unwrap()
    }

    fn path(&self, place: Place) -> PathBuf {
        self.folders.path(place).unwrap()
    }

    fn install(&self, client: ClientId) {
        let marker = Self::entry(client).markers[0];
        fs::create_dir_all(self.path(marker)).unwrap();
    }

    fn write(&self, place: Place, text: &str) {
        let path = self.path(place);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, place: Place) -> String {
        fs::read_to_string(self.path(place)).unwrap()
    }

    fn row(&self, client: ClientId) -> ClientStatus {
        let entry = Self::entry(client);
        status::client_status(entry, entry.declared_method(), &self.folders, &self.server)
    }

    fn connect_file(&self, client: ClientId) -> Result<()> {
        let Method::File(place, format) = Self::entry(client).declared_method() else {
            panic!("a file client");
        };
        connect_file(format, &self.path(place), &self.server, client)
    }

    fn disconnect_file(&self, client: ClientId) -> Result<()> {
        let Method::File(place, format) = Self::entry(client).declared_method() else {
            panic!("a file client");
        };
        safe_write::update(&self.path(place), |current| without_server(format, current))
    }
}

fn server(program: &str, profile: Option<&str>) -> Server {
    Server {
        program: PathBuf::from(program),
        profile: profile.map(PathBuf::from),
    }
}

const CURSOR: Place = Place::Home(".cursor/mcp.json");
const CODEX: Place = Place::Home(".codex/config.toml");

#[test]
fn an_entry_starts_the_helper_for_one_client_and_holds_no_secret() {
    let scratch = Scratch::new();
    scratch.install(ClientId::Cursor);
    scratch.connect_file(ClientId::Cursor).unwrap();
    let value: serde_json::Value = serde_json::from_str(&scratch.read(CURSOR)).unwrap();
    assert_eq!(
        value["mcpServers"]["Huda"],
        serde_json::json!({
            "command": "/opt/huda/huda",
            "args": ["mcp", "stdio", "--client", "cursor"],
        })
    );
}

#[test]
fn a_profile_other_than_the_default_stays_in_the_entry() {
    let chosen = server("/opt/huda/huda", Some("/work/profile"));
    assert_eq!(
        chosen.args(ClientId::Cursor),
        [
            "--profile",
            "/work/profile",
            "mcp",
            "stdio",
            "--client",
            "cursor"
        ]
    );
}

#[test]
fn the_row_follows_the_file_from_missing_to_configured_and_back() {
    let scratch = Scratch::new();
    assert_eq!(
        scratch.row(ClientId::Cursor).state,
        ClientState::NotInstalled
    );
    scratch.install(ClientId::Cursor);
    assert_eq!(scratch.row(ClientId::Cursor).state, ClientState::Available);
    scratch.connect_file(ClientId::Cursor).unwrap();
    assert_eq!(scratch.row(ClientId::Cursor).state, ClientState::Configured);
    scratch.disconnect_file(ClientId::Cursor).unwrap();
    assert_eq!(scratch.row(ClientId::Cursor).state, ClientState::Available);
}

#[test]
fn configured_is_a_statement_about_the_file_and_never_about_the_client() {
    let scratch = Scratch::new();
    scratch.install(ClientId::Cursor);
    scratch.connect_file(ClientId::Cursor).unwrap();
    let row = scratch.row(ClientId::Cursor);
    assert_eq!(row.state, ClientState::Configured);
    assert_eq!(row.activity, Activity::default());
    assert!(!row.activity.is_refused());
}

#[test]
fn an_entry_for_another_install_profile_or_client_needs_repair() {
    let scratch = Scratch::new();
    scratch.install(ClientId::Cursor);
    for args in [
        r#"["mcp", "stdio", "--client", "cursor"]"#,
        r#"["mcp", "stdio", "--client", "kiro"]"#,
        r#"["--profile", "/elsewhere", "mcp", "stdio", "--client", "cursor"]"#,
        r#"["mcp", "stdio"]"#,
    ] {
        let program = if args.contains("cursor") && !args.contains("profile") {
            "/moved/huda"
        } else {
            "/opt/huda/huda"
        };
        scratch.write(
            CURSOR,
            &format!(
                r#"{{ "mcpServers": {{ "Huda": {{ "command": "{program}", "args": {args} }} }} }}"#
            ),
        );
        assert_eq!(
            scratch.row(ClientId::Cursor).state,
            ClientState::NeedsRepair,
            "{args}"
        );
    }
    scratch.connect_file(ClientId::Cursor).unwrap();
    assert_eq!(scratch.row(ClientId::Cursor).state, ClientState::Configured);
}

#[test]
fn a_broken_file_blocks_its_row_says_why_and_stays_as_it_was() {
    let scratch = Scratch::new();
    scratch.install(ClientId::Cursor);
    scratch.write(CURSOR, "{ broken");
    let row = scratch.row(ClientId::Cursor);
    assert_eq!(row.state, ClientState::Blocked);
    assert_eq!(row.error.unwrap().code, "CLIENT_CONFIG_INVALID");
    assert!(scratch.connect_file(ClientId::Cursor).is_err());
    assert_eq!(scratch.read(CURSOR), "{ broken");
}

#[test]
fn one_broken_file_leaves_the_other_rows_alone() {
    let scratch = Scratch::new();
    scratch.install(ClientId::Cursor);
    scratch.install(ClientId::Kiro);
    scratch.write(CURSOR, "{ broken");
    assert_eq!(scratch.row(ClientId::Cursor).state, ClientState::Blocked);
    assert_eq!(scratch.row(ClientId::Kiro).state, ClientState::Available);
}

#[test]
fn another_servers_secret_stays_in_the_file_and_out_of_the_row() {
    let scratch = Scratch::new();
    scratch.install(ClientId::Cursor);
    let original = r#"{ "mcpServers": { "other": { "env": { "TOKEN": "keep-me-secret" } } } }"#;
    scratch.write(CURSOR, original);
    scratch.connect_file(ClientId::Cursor).unwrap();
    assert!(scratch.read(CURSOR).contains("keep-me-secret"));
    let row = serde_json::to_string(&scratch.row(ClientId::Cursor)).unwrap();
    assert!(!row.contains("keep-me-secret"));
    scratch.disconnect_file(ClientId::Cursor).unwrap();
    assert!(scratch.read(CURSOR).contains("keep-me-secret"));
}

#[test]
fn toml_clients_connect_and_disconnect_the_same_way() {
    let scratch = Scratch::new();
    scratch.install(ClientId::Codex);
    scratch.write(CODEX, "# mine\nmodel = \"o4\"\n");
    scratch.connect_file(ClientId::Codex).unwrap();
    assert_eq!(scratch.row(ClientId::Codex).state, ClientState::Configured);
    assert!(scratch.read(CODEX).starts_with("# mine\n"));
    scratch.disconnect_file(ClientId::Codex).unwrap();
    assert_eq!(scratch.row(ClientId::Codex).state, ClientState::Available);
    assert!(scratch.read(CODEX).contains("model = \"o4\""));
}

#[test]
fn every_file_client_reads_back_what_a_write_makes() {
    for entry in &CATALOG {
        if let Method::File(place, _) = entry.declared_method() {
            let scratch = Scratch::new();
            let folder = scratch.path(place);
            fs::create_dir_all(folder.parent().unwrap()).unwrap();
            scratch.install(entry.client);
            scratch.connect_file(entry.client).unwrap();
            assert_eq!(
                scratch.row(entry.client).state,
                ClientState::Configured,
                "{}",
                entry.name
            );
            scratch.disconnect_file(entry.client).unwrap();
            assert_eq!(
                scratch.row(entry.client).state,
                ClientState::Available,
                "{}",
                entry.name
            );
        }
    }
}

#[test]
fn a_manual_or_hosted_row_offers_no_change_and_writes_nothing() {
    let scratch = Scratch::new();
    let goose = scratch.row(ClientId::Goose);
    assert_eq!(goose.state, ClientState::Manual);
    assert!(goose.snippet.unwrap().contains("--client"));
    let hosted = scratch.row(ClientId::ChatgptChat);
    assert_eq!(hosted.state, ClientState::Hosted);
    assert_eq!(hosted.snippet, None);
    for client in [ClientId::Goose, ClientId::ChatgptChat] {
        let entry = Scratch::entry(client);
        assert_eq!(
            apply_connect(entry, &scratch.folders, &scratch.server)
                .unwrap_err()
                .code,
            "CLIENT_NOT_AUTOMATIC"
        );
        assert_eq!(
            apply_disconnect(entry, &scratch.folders).unwrap_err().code,
            "CLIENT_NOT_AUTOMATIC"
        );
    }
}

#[test]
fn a_client_that_is_not_installed_gets_no_file() {
    let scratch = Scratch::new();
    let entry = Scratch::entry(ClientId::Cursor);
    assert!(apply_connect(entry, &scratch.folders, &scratch.server).is_err());
    assert!(!scratch.path(CURSOR).exists());
}

#[test]
fn a_running_client_that_rewrites_its_file_is_refused_and_never_closed() {
    let claude = Scratch::entry(ClientId::ClaudeDesktop);
    assert_eq!(
        require_closed(claude, |process| process == "Claude")
            .unwrap_err()
            .code,
        "CLIENT_RUNNING"
    );
    assert!(require_closed(claude, |_| false).is_ok());
    assert!(require_closed(Scratch::entry(ClientId::Cursor), |_| true).is_ok());
}

#[test]
fn a_path_that_goes_away_is_refused_for_an_entry() {
    for program in [
        "/Volumes/Huda/Huda.app/Contents/MacOS/huda",
        "/private/var/folders/x/AppTranslocation/y/d/Huda.app/Contents/MacOS/huda",
        "relative/huda",
        "/opt/huda\nInjected",
    ] {
        assert!(
            server(program, None).require_lasting().is_err(),
            "{program}"
        );
    }
    // A lasting install on this system; a Windows path is absolute only with its drive.
    let installed = if cfg!(windows) {
        r"C:\Users\me\AppData\Local\Programs\Huda\huda.exe"
    } else {
        "/Applications/Huda.app/Contents/MacOS/huda"
    };
    assert!(
        server(installed, None).require_lasting().is_ok(),
        "{installed}"
    );
}

#[test]
fn a_windows_path_loses_its_verbatim_prefix() {
    assert_eq!(
        without_verbatim_prefix(PathBuf::from(r"\\?\C:\Huda\huda.exe")),
        PathBuf::from(r"C:\Huda\huda.exe")
    );
    assert_eq!(
        without_verbatim_prefix(PathBuf::from("/usr/bin/huda")),
        PathBuf::from("/usr/bin/huda")
    );
}

#[cfg(unix)]
mod own_command {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Stands in for a client's own command: it records what it was asked and writes the file a
    /// real `mcp add` would, so the read-back is the real read-back.
    fn fake_claude(scratch: &Scratch, written: &str) -> PathBuf {
        let program = scratch.home.path().join("claude");
        let log = scratch.home.path().join("calls");
        let status = scratch.path(Place::Home(".claude.json"));
        fs::write(
            &program,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$2\" = add ]; then cat > '{}' <<'JSON'\n{written}\nJSON\nfi\n",
                log.display(),
                status.display(),
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        program
    }

    fn calls(scratch: &Scratch) -> String {
        fs::read_to_string(scratch.home.path().join("calls")).unwrap()
    }

    const GOOD: &str = r#"{ "mcpServers": { "Huda": { "type": "stdio", "command": "/opt/huda/huda", "args": ["mcp", "stdio", "--client", "claude_code"], "env": {} } } }"#;

    #[test]
    fn connecting_passes_the_whole_entry_after_the_separator() {
        let scratch = Scratch::new();
        let program = fake_claude(&scratch, GOOD);
        let status = scratch.path(Place::Home(".claude.json"));
        connect_cli(&program, &status, &scratch.server, ClientId::ClaudeCode).unwrap();
        assert_eq!(
            calls(&scratch).trim(),
            "mcp add --scope user --transport stdio Huda -- /opt/huda/huda mcp stdio --client claude_code"
        );
        assert_eq!(
            scratch.row(ClientId::ClaudeCode).state,
            ClientState::Configured
        );
    }

    #[test]
    fn repairing_removes_the_old_entry_first_because_adding_refuses_a_used_name() {
        let scratch = Scratch::new();
        let program = fake_claude(&scratch, GOOD);
        let status = scratch.path(Place::Home(".claude.json"));
        scratch.write(
            Place::Home(".claude.json"),
            r#"{ "mcpServers": { "Huda": { "command": "/moved/huda" } } }"#,
        );
        assert_eq!(
            scratch.row(ClientId::ClaudeCode).state,
            ClientState::NeedsRepair
        );
        connect_cli(&program, &status, &scratch.server, ClientId::ClaudeCode).unwrap();
        assert!(calls(&scratch).starts_with("mcp remove Huda -s user\nmcp add "));
    }

    #[test]
    fn a_command_that_reports_success_without_the_entry_is_a_failure() {
        let scratch = Scratch::new();
        let program = fake_claude(&scratch, "{}");
        let status = scratch.path(Place::Home(".claude.json"));
        assert_eq!(
            connect_cli(&program, &status, &scratch.server, ClientId::ClaudeCode)
                .unwrap_err()
                .code,
            "CLIENT_CLI_UNAVAILABLE"
        );
    }

    #[test]
    fn huda_never_writes_the_file_a_client_command_owns() {
        let scratch = Scratch::new();
        fs::create_dir_all(scratch.path(Place::Home(".claude"))).unwrap();
        let row = scratch.row(ClientId::ClaudeCode);
        assert_eq!(row.state, ClientState::Available);
        assert!(row.uses_command);
        assert!(row.snippet.unwrap().starts_with("claude mcp add "));
        assert!(!scratch.path(Place::Home(".claude.json")).exists());
    }
}
