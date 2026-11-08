//! What a screen reader is told, read from the accessibility tree of the real app without a
//! window. This proves what VoiceOver receives, not how it speaks it.
use super::*;
use eframe::egui::accesskit::{Live, Node, NodeId, Role, Toggled, TreeUpdate};
use huda::mcp::clients::{Activity, ClientId, ClientState, ClientStatus};
use serde_json::json;
use std::collections::HashMap;

#[derive(Debug)]
struct Spoken {
    id: NodeId,
    role: Role,
    name: String,
    toggled: Option<Toggled>,
    live: Option<Live>,
    level: Option<usize>,
    modal: bool,
}

fn walk(id: NodeId, nodes: &HashMap<NodeId, &Node>, order: &mut Vec<Spoken>) {
    let Some(node) = nodes.get(&id) else { return };
    let name = node.label().or(node.value()).unwrap_or_default().to_owned();
    order.push(Spoken {
        id,
        role: node.role(),
        name,
        toggled: node.toggled(),
        live: node.live(),
        level: node.level(),
        modal: node.is_modal(),
    });
    for child in node.children() {
        walk(*child, nodes, order);
    }
}

/// The tree in the order a screen reader walks it.
fn walked(update: &TreeUpdate) -> Vec<Spoken> {
    let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, node)| (*id, node)).collect();
    let mut order = Vec::new();
    let root = update.tree.as_ref().map_or(update.focus, |tree| tree.root);
    walk(root, &nodes, &mut order);
    order
}

struct App {
    context: egui::Context,
    desktop: Desktop,
    _folder: tempfile::TempDir,
}

impl App {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let context = egui::Context::default();
        theme::apply(&context);
        context.enable_accesskit();
        let profile = Profile::new(folder.path().join("profile")).unwrap();
        let lease = profile.desktop_lease().unwrap();
        let desktop = Desktop::new(profile, false, context.clone(), lease);
        Self {
            context,
            desktop,
            _folder: folder,
        }
    }

    /// Enough frames for a row to measure itself and focus to arrive.
    fn tree(&mut self) -> (Vec<Spoken>, NodeId) {
        let mut last = None;
        for _ in 0..6 {
            let mut output = self.context.run_ui(egui::RawInput::default(), |ui| {
                self.desktop.body(ui);
            });
            output.textures_delta.clear();
            last = output.platform_output.accesskit_update.or(last);
        }
        let update = last.expect("accessibility is enabled");
        (walked(&update), update.focus)
    }

    fn area(&mut self, tab: Tab) -> Vec<Spoken> {
        self.desktop.tab = tab;
        self.tree().0
    }
}

fn client(id: ClientId, name: &'static str, state: ClientState) -> ClientStatus {
    ClientStatus {
        client: id,
        name,
        state,
        note: None,
        error: None,
        path: None,
        snippet: Some("{ }".to_owned()),
        link: Some("https://example.invalid/"),
        uses_command: false,
        activity: Activity::default(),
    }
}

fn clients() -> Vec<ClientStatus> {
    vec![
        client(ClientId::Cursor, "Cursor", ClientState::Available),
        client(ClientId::Kiro, "Kiro", ClientState::Available),
        client(ClientId::VsCode, "VS Code", ClientState::NeedsRepair),
        client(ClientId::Codex, "Codex", ClientState::Configured),
        client(ClientId::Goose, "Goose", ClientState::Manual),
    ]
}

fn operable(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::CheckBox
            | Role::Switch
            | Role::ComboBox
            | Role::TextInput
            | Role::MultilineTextInput
            | Role::Link
    )
}

fn position(tree: &[Spoken], name: &str) -> usize {
    tree.iter()
        .position(|node| node.name == name)
        .unwrap_or_else(|| panic!("nothing called {name:?}"))
}

#[test]
fn every_control_in_every_area_has_a_name() {
    let mut app = App::new();
    app.desktop.clients.show_rows(clients());
    for tab in Tab::ORDER {
        for node in app.area(tab) {
            if operable(node.role) {
                assert!(!node.name.trim().is_empty(), "{tab:?}: unnamed {node:?}");
                assert_ne!(node.name, "…", "{tab:?}: a glyph is not a name");
            }
        }
    }
}

#[test]
fn a_list_of_clients_never_repeats_a_controls_name() {
    let mut app = App::new();
    app.desktop.clients.show_rows(clients());
    let tree = app.area(Tab::Clients);
    let mut names: Vec<_> = tree
        .iter()
        .filter(|node| operable(node.role))
        .map(|node| node.name.as_str())
        .collect();
    let all = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all, "{names:?}");
    for name in [
        "Connect Cursor",
        "Repair VS Code",
        "Disconnect Codex",
        "More for Goose",
    ] {
        position(&tree, name);
    }
}

#[test]
fn a_row_is_read_in_the_order_it_is_seen() {
    let mut app = App::new();
    app.desktop.clients.show_rows(clients());
    let tree = app.area(Tab::Clients);
    let (name, status) = (position(&tree, "Cursor"), position(&tree, "Ready"));
    let (action, more) = (
        position(&tree, "Connect Cursor"),
        position(&tree, "More for Cursor"),
    );
    assert!(
        name < status && status < action && action < more,
        "{tree:#?}"
    );
    let tree = app.area(Tab::Space);
    assert!(position(&tree, "Set up this profile") < position(&tree, "Initialize Profile"));
}

#[test]
fn each_area_has_one_top_heading_and_it_is_the_title() {
    let mut app = App::new();
    app.desktop.clients.show_rows(clients());
    for tab in Tab::ORDER {
        let tree = app.area(tab);
        let top: Vec<_> = tree
            .iter()
            .filter(|node| node.role == Role::Heading && node.level == Some(1))
            .map(|node| node.name.as_str())
            .collect();
        assert_eq!(top, [tab.title()], "{tab:?}");
    }
}

#[test]
fn a_section_title_is_a_heading_below_the_title() {
    let mut app = App::new();
    let tree = app.area(Tab::Settings);
    let startup = tree.iter().find(|node| node.name == "Startup").unwrap();
    assert_eq!((startup.role, startup.level), (Role::Heading, Some(2)));
}

#[test]
fn the_sidebar_is_navigation_and_says_where_you_are() {
    let mut app = App::new();
    let tree = app.area(Tab::Ask);
    assert!(tree.iter().any(|node| node.role == Role::Navigation));
    let selected: Vec<_> = tree
        .iter()
        .filter(|node| node.toggled == Some(Toggled::True))
        .map(|node| node.name.as_str())
        .collect();
    assert_eq!(selected, ["Ask Huda"]);
}

#[test]
fn a_compact_sidebar_keeps_its_names() {
    let mut app = App::new();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(640.0, 480.0),
        )),
        ..Default::default()
    };
    let mut last = None;
    for _ in 0..3 {
        let mut output = app.context.run_ui(raw.clone(), |ui| app.desktop.body(ui));
        output.textures_delta.clear();
        last = output.platform_output.accesskit_update.or(last);
    }
    let tree = walked(&last.unwrap());
    for tab in Tab::ORDER {
        position(&tree, tab.title());
    }
}

#[test]
fn a_setting_is_a_switch_that_reports_its_state() {
    let mut app = App::new();
    app.desktop.keep_running = true;
    let tree = app.area(Tab::Settings);
    let node = tree
        .iter()
        .find(|node| node.name == "Keep Running After Close" && node.role == Role::Switch)
        .unwrap();
    assert_eq!(node.toggled, Some(Toggled::True));
}

#[test]
fn a_sheet_is_a_modal_dialog_and_focus_lands_on_cancel() {
    let mut app = App::new();
    let rows = clients();
    app.desktop.clients.show_sheet(&rows[3]);
    app.desktop.clients.show_rows(rows);
    app.desktop.tab = Tab::Clients;
    let (tree, focus) = app.tree();
    let dialog = tree.iter().find(|node| node.role == Role::Dialog).unwrap();
    assert_eq!(dialog.name, "Disconnect Codex");
    assert!(dialog.modal);
    let focused = tree.iter().find(|node| node.id == focus).unwrap();
    assert_eq!(focused.name, "Cancel");
}

#[test]
fn a_problem_interrupts_and_a_result_waits_its_turn() {
    let mut app = App::new();
    app.desktop
        .results
        .set(Err(huda::error::Error::new("AUTH_REQUIRED", "Unlock it")));
    let tree = app.area(Tab::Diagnostics);
    let alert = tree.iter().find(|node| node.role == Role::Alert).unwrap();
    assert_eq!(alert.name, "AUTH_REQUIRED: Unlock it");
    assert_eq!(alert.live, Some(Live::Assertive));
    app.desktop.results.message("Done");
    let tree = app.area(Tab::Diagnostics);
    let status = tree.iter().find(|node| node.role == Role::Status).unwrap();
    assert_eq!(status.live, Some(Live::Polite));
}

#[test]
fn a_negation_is_heard_before_the_parties_it_changes() {
    let mut app = App::new();
    app.desktop.results.set(Ok(json!({
        "entities": [{"entity_id": "e", "labels": ["rustls"]}],
        "relations": [{
            "relation": {
                "relation_type": "core:DEPENDS_ON@1",
                "roles": {"dependency": [{"kind": "entity", "entity_id": "e"}]},
                "polarity": "negative",
                "modality": "planned",
            },
            "assertions": [{"publication_status": "active", "stale": false}],
        }],
    })));
    let tree = app.area(Tab::Space);
    assert!(position(&tree, "Not, Planned, Depends on") < position(&tree, "rustls"));
}

#[test]
fn settings_say_what_huda_is_under_its_one_name() {
    let mut app = App::new();
    let tree = app.area(Tab::Settings);
    let about = position(&tree, "About");
    // The sidebar also says Huda; the About section's own name comes after its heading.
    let name = about
        + tree[about..]
            .iter()
            .position(|node| node.name == huda::APP_NAME)
            .expect("About names Huda");
    let description = position(&tree, huda::DESCRIPTION);
    let version = position(
        &tree,
        &format!("Version {} · {}", huda::VERSION, huda::APP_ID),
    );
    assert!(about < name && name < description && description < version);
    assert!(tree
        .iter()
        .all(|node| !node.name.contains("huda-desktop") && !node.name.contains("huda-app")));
}
