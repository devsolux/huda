//! What this profile has seen of each client. A configuration entry proves only that a file holds
//! it, and a client that starts Huda may still refuse it, so only a tool call counts as use. A
//! client that opens with a handshake Huda does not speak is recorded too: that is the one reason
//! a configured client fails which Huda can see for itself.
//!
//! Each client has one small private file holding two times. No request content is kept. This is
//! a convenience for the status list, never a condition for serving, so a failure to read or
//! write it changes nothing else.
use super::catalog::ClientId;
use crate::runtime::Profile;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A busy client calls many times a minute; the file only has to answer "when, roughly".
const QUIET_MS: u64 = 60_000;

/// When a client last used Huda, and when Huda last refused its handshake. Milliseconds since 1970.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub used_ms: Option<u64>,
    pub refused_ms: Option<u64>,
}

impl Activity {
    /// Whether the latest thing seen of this client is a refused handshake.
    pub fn is_refused(&self) -> bool {
        self.refused_ms > self.used_ms
    }
}

fn state_name(client: ClientId) -> String {
    format!("mcp-client-{}", client.id())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// What is recorded for `client`. An unreadable file reads as nothing seen.
pub fn read(profile: &Profile, client: ClientId) -> Activity {
    profile
        .read_state(&state_name(client))
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Records one client's use of `huda mcp stdio`.
#[derive(Debug)]
pub struct Observer {
    profile: Profile,
    client: ClientId,
    /// When a tool call was last written. A refusal does not count: a client that falls back to
    /// this protocol right after one must not lose the call that shows it works.
    last_used_ms: AtomicU64,
}

impl Observer {
    /// The observer of a client named on a command line. A configuration can hold any name, and
    /// a name Huda does not know is left out, so no file name is ever built from input.
    pub fn new(profile: &Profile, name: &str) -> Option<Self> {
        Some(Self {
            profile: profile.clone(),
            client: ClientId::from_id(name)?,
            last_used_ms: AtomicU64::new(0),
        })
    }

    fn record(&self, change: impl FnOnce(&mut Activity, u64)) {
        let mut activity = read(&self.profile, self.client);
        let now = now_ms();
        change(&mut activity, now);
        if let Ok(text) = serde_json::to_string(&activity) {
            let _ = self.profile.write_state(&state_name(self.client), &text);
        }
    }

    /// The client called a tool.
    pub fn used(&self) {
        let last = self.last_used_ms.load(Ordering::Acquire);
        let now = now_ms();
        if now.saturating_sub(last) >= QUIET_MS {
            self.last_used_ms.store(now, Ordering::Release);
            self.record(|activity, now| activity.used_ms = Some(now));
        }
    }

    /// The client opened with a handshake Huda refuses.
    pub fn refused(&self) {
        self.record(|activity, now| activity.refused_ms = Some(now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> (tempfile::TempDir, Profile) {
        let folder = tempfile::tempdir().unwrap();
        let profile = Profile::new(folder.path().join("profile")).unwrap();
        (folder, profile)
    }

    #[test]
    fn nothing_seen_reads_as_nothing_and_writes_nothing() {
        let (_folder, profile) = profile();
        assert_eq!(read(&profile, ClientId::Cursor), Activity::default());
        assert_eq!(std::fs::read_dir(profile.directory()).unwrap().count(), 0);
    }

    #[test]
    fn a_name_huda_does_not_know_is_left_out() {
        let (_folder, profile) = profile();
        assert!(Observer::new(&profile, "../../escape").is_none());
        assert!(Observer::new(&profile, "cursor").is_some());
    }

    #[test]
    fn a_client_that_falls_back_after_a_refusal_shows_as_working() {
        let (_folder, profile) = profile();
        let observer = Observer::new(&profile, "cursor").unwrap();
        observer.refused();
        assert!(read(&profile, ClientId::Cursor).is_refused());
        std::thread::sleep(std::time::Duration::from_millis(2));
        observer.used();
        let activity = read(&profile, ClientId::Cursor);
        assert!(activity.used_ms.is_some());
        assert!(!activity.is_refused());
    }

    #[test]
    fn a_busy_client_does_not_rewrite_the_file_for_every_call() {
        let (_folder, profile) = profile();
        let observer = Observer::new(&profile, "cursor").unwrap();
        observer.used();
        let first = read(&profile, ClientId::Cursor);
        std::thread::sleep(std::time::Duration::from_millis(2));
        observer.used();
        assert_eq!(read(&profile, ClientId::Cursor), first);
    }

    #[test]
    fn clients_are_recorded_apart_and_a_broken_file_reads_as_nothing() {
        let (_folder, profile) = profile();
        Observer::new(&profile, "cursor").unwrap().used();
        assert_eq!(read(&profile, ClientId::Kiro), Activity::default());
        profile
            .write_state(&state_name(ClientId::Kiro), "not json")
            .unwrap();
        assert_eq!(read(&profile, ClientId::Kiro), Activity::default());
    }

    #[test]
    fn the_file_holds_no_more_than_the_two_times() {
        let (_folder, profile) = profile();
        Observer::new(&profile, "cursor").unwrap().refused();
        let text = profile
            .read_state(&state_name(ClientId::Cursor))
            .unwrap()
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert!(text.len() < 256);
    }
}
