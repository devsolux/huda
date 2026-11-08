//! A profile that was never set up, or that lost its credential: the app offers the one step that
//! fixes it and never sends a request that can only fail with a misleading reason. These tests set
//! the app's state directly, so no test reads or changes a real OS secret store.
use super::*;

struct App {
    context: egui::Context,
    desktop: Desktop,
    folder: tempfile::TempDir,
}

impl App {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let context = egui::Context::default();
        theme::apply(&context);
        let profile = Profile::new(folder.path().join("profile")).unwrap();
        let lease = profile.desktop_lease().unwrap();
        let desktop = Desktop::new(profile, false, context.clone(), lease);
        Self {
            context,
            desktop,
            folder,
        }
    }

    /// Whether the area on screen offers setup, read from what a screen reader is given.
    fn offers_setup(&mut self, tab: Tab) -> bool {
        self.offers(tab, "Initialize Profile")
    }
    fn offers(&mut self, tab: Tab, action: &str) -> bool {
        self.desktop.tab = tab;
        self.context.enable_accesskit();
        let mut last = None;
        for _ in 0..3 {
            let mut output = self
                .context
                .run_ui(egui::RawInput::default(), |ui| self.desktop.body(ui));
            output.textures_delta.clear();
            last = output.platform_output.accesskit_update.or(last);
        }
        last.unwrap()
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some(action))
    }
    /// A profile with a database, as after `huda init`; its credential is the app's business.
    fn with_database() -> Self {
        let app = Self::new();
        drop(huda::storage::Store::open(app.desktop.client.profile.clone()).unwrap());
        app
    }
}
const ASKS_THE_CORE: [Tab; 6] = [
    Tab::Space,
    Tab::Ask,
    Tab::Editor,
    Tab::Sources,
    Tab::Integrations,
    Tab::Diagnostics,
];

#[test]
fn a_request_before_setup_is_not_sent_and_says_why() {
    let mut app = App::new();
    app.desktop.submit("status", json!({}));
    assert!(app.desktop.pending.is_none(), "nothing may reach the Core");
    let shown = app.desktop.results.shown();
    assert!(shown.contains("not set up"), "{shown}");
    assert!(!shown.contains("Unlock"), "{shown}");
    assert!(!app.folder.path().join("profile/core.sock").exists());
}

#[test]
fn every_area_that_asks_the_core_offers_setup_and_the_others_do_not() {
    let mut app = App::new();
    for tab in ASKS_THE_CORE {
        assert!(app.offers_setup(tab), "{tab:?}");
    }
    for tab in [Tab::Clients, Tab::Settings] {
        assert!(!app.offers_setup(tab), "{tab:?}");
    }
}

#[test]
fn a_failed_setup_shows_the_helpers_own_reason() {
    let refused = br#"{"error":{"code":"VERSION_CONFLICT","message":"Profile is already initialized; credentials were not overwritten"}}"#;
    let error = helper_error(refused, Setup::Initialize);
    assert_eq!(error.code, "VERSION_CONFLICT");
    assert!(error.message.contains("already initialized"));
    let unreadable = helper_error(b"Segmentation fault\n", Setup::Initialize);
    assert_eq!(unreadable.code, "SETUP_FAILED");
    assert!(unreadable.message.contains("huda init"));
    let restore = helper_error(b"", Setup::Restore);
    assert!(
        restore.message.contains("huda credentials restore"),
        "{}",
        restore.message
    );
    let huge = vec![b'x'; 1_000_000];
    assert_eq!(helper_error(&huge, Setup::Restore).code, "SETUP_FAILED");
}

#[test]
fn a_lost_credential_offers_restore_access_where_initialize_would_fail() {
    let mut app = App::with_database();
    app.desktop.answered(Answer::Lost);
    assert!(app.desktop.results.shown().contains("Restore Access"));
    for tab in ASKS_THE_CORE {
        assert!(app.offers(tab, "Restore Access"), "{tab:?}");
        assert!(
            !app.offers_setup(tab),
            "{tab:?}: huda init refuses an initialized profile"
        );
    }
    for tab in [Tab::Clients, Tab::Settings] {
        assert!(!app.offers(tab, "Restore Access"), "{tab:?}");
    }
}

#[test]
fn a_locked_secret_store_says_so_and_offers_no_setup() {
    let mut app = App::with_database();
    app.desktop.answered(Answer::Lost);
    let locked = huda::error::Error::new(
        "AUTH_REQUIRED",
        "The OS secret store refused access to Huda's credential. Unlock it",
    );
    app.desktop.answered(Answer::Done(Err(locked)));
    assert!(app.desktop.results.shown().contains("Unlock"));
    assert!(!app.offers(Tab::Space, "Restore Access"));
    assert!(!app.offers_setup(Tab::Space));
}

#[test]
fn finished_setup_reads_the_new_credential_and_a_half_finished_one_offers_restore() {
    let mut app = App::with_database();
    app.desktop.answered(Answer::Lost);
    app.desktop.pending_setup = Some(Setup::Restore);
    app.desktop
        .answered(Answer::Done(Ok(json!({"ready": true}))));
    assert!(!app.desktop.needs_restore);
    assert!(app.desktop.results.shown().contains("ready"));

    // Initialization that stopped after creating the database is finished by Restore Access.
    app.desktop.pending_setup = Some(Setup::Initialize);
    let failed = huda::error::Error::new("SECRET_STORE_UNAVAILABLE", "could not save");
    app.desktop.answered(Answer::Done(Err(failed)));
    assert!(app.offers(Tab::Space, "Restore Access"));
}
