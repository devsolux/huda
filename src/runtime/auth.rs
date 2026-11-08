//! Request authentication is independent of model/context content.
use super::Profile;
use crate::error::{require, Error, Result};
use crate::relation::{identifier, random_id, Caller};
use crate::storage::{audit, audit_as, tick, Store};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrincipalRequest {
    pub actor: String,
    pub read_scopes: BTreeSet<String>,
    pub write_scopes: BTreeSet<String>,
    pub permissions: BTreeSet<String>,
    pub expected_policy_epoch: u64,
}
impl PrincipalRequest {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.actor)?;
        require(
            !self.read_scopes.is_empty()
                && self.read_scopes.len() <= 32
                && self.write_scopes.is_subset(&self.read_scopes),
            "INVALID_INPUT",
            "Invalid scoped capability",
        )?;
        for scope in &self.read_scopes {
            identifier(scope)?;
        }
        for permission in &self.permissions {
            require(
                matches!(
                    permission.as_str(),
                    "read"
                        | "write"
                        | "ingest"
                        | "registry"
                        | "policy"
                        | "admin"
                        | "jobs"
                        | "expand"
                        | "bridge"
                        | "review"
                ),
                "INVALID_INPUT",
                "Unknown permission",
            )?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeRequest {
    pub actor: String,
    pub expected_version: u64,
    pub expected_policy_epoch: u64,
}

pub(crate) fn token_hash(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}
impl Store {
    pub fn authenticate(&self, token: &str) -> Result<Caller> {
        require(
            token.len() >= 32 && token.len() <= 256,
            "AUTH_REQUIRED",
            "Valid credentials are required",
        )?;
        let data:Option<(String,String,String,String)>=self.conn.query_row("SELECT actor,read_scopes_json,write_scopes_json,permissions_json FROM principals WHERE token_hash=?1 AND enabled=1",[token_hash(token)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let (actor, read, write, permissions) =
            data.ok_or_else(|| Error::new("AUTH_REQUIRED", "Valid credentials are required"))?;
        Ok(Caller {
            actor,
            read_scopes: serde_json::from_str(&read)?,
            write_scopes: serde_json::from_str(&write)?,
            permissions: serde_json::from_str(&permissions)?,
            deadline_ms: crate::relation::now_ms() + 30_000,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
    pub fn has_principals(&self) -> Result<bool> {
        Ok(self
            .conn
            .prepare("SELECT 1 FROM principals LIMIT 1")?
            .exists([])?)
    }
    /// Trusted embedding/bootstrap interface; never exposed as a remote operation.
    pub fn bootstrap_credentials(&mut self, owner_token: &str, mcp_token: &str) -> Result<()> {
        require(
            owner_token.len() >= 32 && mcp_token.len() >= 32 && owner_token != mcp_token,
            "INVALID_INPUT",
            "Bootstrap needs distinct secure tokens",
        )?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require(
            !tx.prepare("SELECT 1 FROM principals LIMIT 1")?.exists([])?,
            "VERSION_CONFLICT",
            "Credentials are already initialized",
        )?;
        for (actor, token, permissions) in [
            (
                "owner",
                owner_token,
                vec![
                    "read", "write", "ingest", "registry", "policy", "admin", "jobs", "expand",
                    "bridge", "review",
                ],
            ),
            (
                "mcp",
                mcp_token,
                vec!["read", "write", "ingest", "jobs", "expand"],
            ),
        ] {
            tx.execute("INSERT INTO principals(actor,token_hash,read_scopes_json,write_scopes_json,permissions_json,enabled,version) VALUES(?1,?2,'[\"local\"]','[\"local\"]',?3,1,1)",params![actor,token_hash(token),serde_json::to_string(&permissions)?])?;
        }
        tick(&tx)?;
        tx.execute(
            "UPDATE system_meta SET policy_epoch=policy_epoch+1 WHERE singleton=1",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Read-only preflight for `add_principal`, run before any OS secret-store write so that a
    /// refused request leaves no orphan credential behind.
    pub fn check_new_principal(&self, caller: &Caller, request: &PrincipalRequest) -> Result<()> {
        caller.require("admin")?;
        request.validate()?;
        crate::storage::recheck(&self.conn, caller, request.expected_policy_epoch)?;
        require(
            !self
                .conn
                .prepare("SELECT 1 FROM principals WHERE actor=?1")?
                .exists([&request.actor])?,
            "VERSION_CONFLICT",
            "Actor already exists; credentials are not overwritten",
        )
    }
    pub fn add_principal(
        &mut self,
        caller: &Caller,
        request: PrincipalRequest,
        token: &str,
    ) -> Result<()> {
        caller.require("admin")?;
        request.validate()?;
        let tx = self.begin(caller, request.expected_policy_epoch)?;
        require(
            !tx.prepare("SELECT 1 FROM principals WHERE actor=?1")?
                .exists([&request.actor])?,
            "VERSION_CONFLICT",
            "Actor already exists; credentials are not overwritten",
        )?;
        let t = tick(&tx)?;
        tx.execute("INSERT INTO principals(actor,token_hash,read_scopes_json,write_scopes_json,permissions_json,enabled,version) VALUES(?1,?2,?3,?4,?5,1,1)",params![request.actor,token_hash(token),serde_json::to_string(&request.read_scopes)?,serde_json::to_string(&request.write_scopes)?,serde_json::to_string(&request.permissions)?])?;
        tx.execute(
            "UPDATE system_meta SET policy_epoch=policy_epoch+1 WHERE singleton=1",
            [],
        )?;
        audit(&tx, t, caller, "add_principal", &request.actor)?;
        caller.check()?;
        tx.commit()?;
        Ok(())
    }
    pub fn revoke_principal(&mut self, caller: &Caller, request: RevokeRequest) -> Result<()> {
        caller.require("admin")?;
        require(
            request.actor != "owner",
            "PERMISSION_DENIED",
            "The bootstrap owner cannot be revoked through this interface",
        )?;
        let tx = self.begin(caller, request.expected_policy_epoch)?;
        let changed=tx.execute("UPDATE principals SET enabled=0,version=version+1 WHERE actor=?1 AND version=?2 AND enabled=1",params![request.actor,request.expected_version])?;
        require(
            changed == 1,
            "VERSION_CONFLICT",
            "Principal version/state changed",
        )?;
        let t = tick(&tx)?;
        tx.execute(
            "UPDATE system_meta SET policy_epoch=policy_epoch+1 WHERE singleton=1",
            [],
        )?;
        audit(&tx, t, caller, "revoke_principal", &request.actor)?;
        caller.check()?;
        tx.commit()?;
        Ok(())
    }
    /// Every principal that can still authenticate, with the hash its credential must match.
    fn enabled_principals(&self) -> Result<Vec<(String, String)>> {
        let mut statement = self
            .conn
            .prepare("SELECT actor,token_hash FROM principals WHERE enabled=1 ORDER BY actor")?;
        let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
    /// Gives an enabled principal a new credential and keeps its scopes and permissions. `issue`
    /// writes the OS secret store inside the write transaction, so a store that refuses the write
    /// leaves the database unchanged. A revoked principal is never given a credential again: that
    /// would undo its revocation.
    fn reissue_credential(
        &mut self,
        actor: &str,
        issue: impl FnOnce() -> Result<String>,
    ) -> Result<()> {
        identifier(actor)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u64 = tx
            .query_row(
                "SELECT version FROM principals WHERE actor=?1 AND enabled=1",
                [actor],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| {
                Error::new(
                    "NOT_FOUND",
                    "No enabled principal has this actor. A revoked principal is never given a credential again",
                )
            })?;
        let token = issue()?;
        let changed = tx.execute(
            "UPDATE principals SET token_hash=?1,version=version+1 WHERE actor=?2 AND version=?3 AND enabled=1",
            params![token_hash(&token), actor, version],
        )?;
        require(
            changed == 1,
            "VERSION_CONFLICT",
            "Principal version/state changed",
        )?;
        let t = tick(&tx)?;
        tx.execute(
            "UPDATE system_meta SET policy_epoch=policy_epoch+1 WHERE singleton=1",
            [],
        )?;
        audit_as(&tx, t, LOCAL_OPERATOR, "reissue_credential", actor)?;
        tx.commit()?;
        Ok(())
    }
}

/// The actor recorded for local recovery. It is not a valid identifier, so no principal can ever
/// carry this name and be mistaken for it.
pub const LOCAL_OPERATOR: &str = "@local-operator";

/// What `restore_access` did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Restored {
    /// Initialization had stopped before the profile's principals were saved; it is now finished.
    pub initialized: bool,
    /// Principals given a new credential, because theirs was missing or no longer accepted.
    pub reissued: Vec<String>,
    /// Principals whose stored credential still works. They were not changed.
    pub kept: Vec<String>,
}

/// Gives every enabled principal of this profile a working credential again, after the OS secret
/// store lost one (a reset keychain, a moved profile folder) or one stopped matching (an
/// interrupted reissue, a database copied from elsewhere). Credentials that still work are kept.
///
/// Every credential is read before anything is written. A store that is locked or failing stops the
/// repair, so a store that cannot be read is never mistaken for one that lost a credential.
///
/// This is local recovery under the Core lease that `store` holds, never a Core operation: whoever
/// lost a credential cannot authenticate. Its trust is the OS account that owns the profile, the
/// same as for `huda init`, and each reissue is audited.
pub fn restore_access(store: &mut Store, profile: &Profile) -> Result<Restored> {
    if !store.has_principals()? {
        let owner = OsSecrets::create(profile, "owner")?;
        let mcp = OsSecrets::create(profile, "mcp")?;
        store.bootstrap_credentials(&owner, &mcp)?;
        return Ok(Restored {
            initialized: true,
            reissued: vec!["mcp".into(), "owner".into()],
            kept: Vec::new(),
        });
    }
    let (mut reissued, mut kept) = (Vec::new(), Vec::new());
    for (actor, hash) in store.enabled_principals()? {
        match OsSecrets::read(profile, &actor)? {
            Some(token) if token_hash(&token) == hash => kept.push(actor),
            _ => reissued.push(actor),
        }
    }
    for actor in &reissued {
        store.reissue_credential(actor, || OsSecrets::create(profile, actor))?;
    }
    Ok(Restored {
        initialized: false,
        reissued,
        kept,
    })
}

/// Replaces one enabled principal's credential, for example after it may have leaked. The old one
/// stops working at once; clients read the new one on their next request. Local recovery, like
/// `restore_access`.
pub fn rotate_credential(store: &mut Store, profile: &Profile, actor: &str) -> Result<()> {
    store.reissue_credential(actor, || OsSecrets::create(profile, actor))
}

/// Why the OS secret store could not do what was asked. It says nothing about the credential.
/// Which faults can occur depends on the build: one without `os-secrets` reports only `NotBuilt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    /// The store is locked, or the person declined the system prompt.
    #[cfg_attr(not(feature = "os-secrets"), allow(dead_code))]
    Refused,
    /// The store answered with an error.
    #[cfg_attr(not(feature = "os-secrets"), allow(dead_code))]
    Failed,
    /// No store runs in this session, such as Linux without a Secret Service.
    #[cfg_attr(not(feature = "os-secrets"), allow(dead_code))]
    Unavailable,
    /// This build has no secret store support.
    #[cfg_attr(feature = "os-secrets", allow(dead_code))]
    NotBuilt,
}

#[cfg(feature = "os-secrets")]
fn fault_of(error: &keyring::v1::Error) -> Fault {
    match error {
        keyring::v1::Error::NoStorageAccess(_) => Fault::Refused,
        keyring::v1::Error::NoDefaultStore => Fault::Unavailable,
        _ => Fault::Failed,
    }
}

/// Each case says what to do next. A client that cannot read its credential cannot authenticate,
/// so reads fail as `AUTH_REQUIRED`, the code clients key on. A credential that cannot be saved is
/// a local dependency failing for a caller who may well be authenticated, so it is never reported
/// as an authentication failure.
fn store_error(fault: Fault, saving: bool) -> Error {
    let code = if saving {
        "SECRET_STORE_UNAVAILABLE"
    } else {
        "AUTH_REQUIRED"
    };
    let message = match (fault, saving) {
        (Fault::Refused, false) => "The OS secret store refused access to Huda's credential. Unlock it, or allow Huda when the system asks, then try again",
        (Fault::Refused, true) => "The OS secret store refused to save Huda's credential. Unlock it, or allow Huda when the system asks, then try again",
        (Fault::Failed, false) => "The OS secret store could not read Huda's credential. Make sure it is running and unlocked, then try again",
        (Fault::Failed, true) => "The OS secret store could not save Huda's credential. Make sure it is running and unlocked, then try again",
        (Fault::Unavailable, _) => "No OS secret store is available in this session, and Huda keeps credentials nowhere else. Use the macOS Keychain, Windows Credential Manager, or on Linux a Secret Service such as GNOME Keyring or KWallet",
        (Fault::NotBuilt, _) => "This build has no OS secret store support. Build with the os-secrets feature to use native clients",
    };
    Error::new(code, message)
}

/// The OS secret store itself. Unit tests get an in-memory stand-in, so no test can read or change
/// the real keychain of the person running them.
mod vault {
    #[cfg(not(test))]
    use super::Fault;
    #[cfg(all(not(test), feature = "os-secrets"))]
    fn entry(service: &str, account: &str) -> Result<keyring::v1::Entry, Fault> {
        keyring::v1::Entry::new(service, account).map_err(|error| super::fault_of(&error))
    }
    #[cfg(all(not(test), feature = "os-secrets"))]
    pub(super) fn read(service: &str, account: &str) -> Result<Option<String>, Fault> {
        match entry(service, account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::v1::Error::NoEntry) => Ok(None),
            Err(error) => Err(super::fault_of(&error)),
        }
    }
    #[cfg(all(not(test), feature = "os-secrets"))]
    pub(super) fn write(service: &str, account: &str, secret: &str) -> Result<(), Fault> {
        entry(service, account)?
            .set_password(secret)
            .map_err(|error| super::fault_of(&error))
    }
    #[cfg(all(not(test), not(feature = "os-secrets")))]
    pub(super) fn read(_: &str, _: &str) -> Result<Option<String>, Fault> {
        Err(Fault::NotBuilt)
    }
    #[cfg(all(not(test), not(feature = "os-secrets")))]
    pub(super) fn write(_: &str, _: &str, _: &str) -> Result<(), Fault> {
        Err(Fault::NotBuilt)
    }
    #[cfg(test)]
    pub(super) use super::memory::{read, write};
}

/// The stand-in secret store for unit tests, with controls to lose, refuse or count credentials.
#[cfg(test)]
pub(crate) mod memory {
    use super::{Fault, OsSecrets, Profile};
    use std::collections::BTreeMap;
    use std::sync::{Mutex, PoisonError};

    #[derive(Default)]
    struct Slot {
        secret: Option<String>,
        fault: Option<Fault>,
        reads: usize,
    }
    // Keys carry the profile fingerprint, and each test uses its own temporary profile.
    static SLOTS: Mutex<BTreeMap<(String, String), Slot>> = Mutex::new(BTreeMap::new());

    fn with<T>(service: &str, account: &str, change: impl FnOnce(&mut Slot) -> T) -> T {
        let mut slots = SLOTS.lock().unwrap_or_else(PoisonError::into_inner);
        change(slots.entry((service.into(), account.into())).or_default())
    }
    fn slot<T>(profile: &Profile, actor: &str, change: impl FnOnce(&mut Slot) -> T) -> T {
        with(&OsSecrets::service(profile), actor, change)
    }
    pub(super) fn read(service: &str, account: &str) -> Result<Option<String>, Fault> {
        with(service, account, |slot| {
            slot.reads += 1;
            slot.fault.map_or_else(|| Ok(slot.secret.clone()), Err)
        })
    }
    pub(super) fn write(service: &str, account: &str, secret: &str) -> Result<(), Fault> {
        with(service, account, |slot| match slot.fault {
            Some(fault) => Err(fault),
            None => {
                slot.secret = Some(secret.into());
                Ok(())
            }
        })
    }
    pub(crate) fn stored(profile: &Profile, actor: &str) -> Option<String> {
        slot(profile, actor, |slot| slot.secret.clone())
    }
    pub(crate) fn put(profile: &Profile, actor: &str, secret: &str) {
        slot(profile, actor, |slot| slot.secret = Some(secret.into()));
    }
    pub(crate) fn lose(profile: &Profile, actor: &str) {
        slot(profile, actor, |slot| slot.secret = None);
    }
    pub(crate) fn break_with(profile: &Profile, actor: &str, fault: Option<Fault>) {
        slot(profile, actor, |slot| slot.fault = fault);
    }
    pub(crate) fn reads(profile: &Profile, actor: &str) -> usize {
        slot(profile, actor, |slot| slot.reads)
    }
}

/// Secrets never enter SQLite, config JSON, command-line arguments, or logs.
pub struct OsSecrets;
impl OsSecrets {
    fn service(profile: &Profile) -> String {
        format!("huda.{}", profile.fingerprint())
    }
    /// The stored credential, or `None` when the store holds none for this actor. A store that is
    /// locked or failing is an error, never `None`.
    pub fn read(profile: &Profile, actor: &str) -> Result<Option<String>> {
        vault::read(&Self::service(profile), actor).map_err(|fault| store_error(fault, false))
    }
    /// The credential a client sends. A missing one says how to get access back.
    pub fn get(profile: &Profile, actor: &str) -> Result<String> {
        Self::read(profile, actor)?.ok_or_else(|| missing(profile, actor))
    }
    /// Issues a fresh credential. An entry already stored under this name (a deleted profile at the
    /// same path, an interrupted init, a credential being replaced) is overwritten, never adopted:
    /// it may have leaked. Callers hold the Core lease (`init`, recovery) or the store lock
    /// (`add_principal`), so no concurrent issuer can race the replacement.
    pub fn create(profile: &Profile, actor: &str) -> Result<String> {
        let token = random_id("cap")?;
        vault::write(&Self::service(profile), actor, &token)
            .map_err(|fault| store_error(fault, true))?;
        Ok(token)
    }
}

/// A profile that was never set up and one that lost a credential need different next steps.
/// Whether the database file exists tells them apart without opening it.
fn missing(profile: &Profile, actor: &str) -> Error {
    if profile.database().exists() {
        Error::new(
            "AUTH_REQUIRED",
            &format!("The OS secret store holds no {actor} credential for this profile. Restore access with huda credentials restore, or with Restore Access in the desktop"),
        )
    } else {
        Error::new(
            "AUTH_REQUIRED",
            "This profile is not initialized yet. Initialize it with huda init, or with Initialize Profile in the desktop",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Store;

    struct Fixture {
        _folder: tempfile::TempDir,
        profile: Profile,
        store: Store,
    }
    /// An initialized profile whose credentials live in the stand-in store.
    fn initialized() -> Fixture {
        let folder = tempfile::tempdir().unwrap();
        let profile = Profile::new(folder.path().join("profile")).unwrap();
        let mut store = Store::open(profile.clone()).unwrap();
        let owner = OsSecrets::create(&profile, "owner").unwrap();
        let mcp = OsSecrets::create(&profile, "mcp").unwrap();
        store.bootstrap_credentials(&owner, &mcp).unwrap();
        Fixture {
            _folder: folder,
            profile,
            store,
        }
    }
    fn works(f: &Fixture, actor: &str) -> bool {
        memory::stored(&f.profile, actor)
            .is_some_and(|token| f.store.authenticate(&token).is_ok_and(|c| c.actor == actor))
    }
    fn epoch_and_events(store: &Store) -> (u64, u64) {
        let conn = store.open_reader().unwrap();
        (
            conn.query_row("SELECT policy_epoch FROM system_meta", [], |r| r.get(0))
                .unwrap(),
            conn.query_row(
                "SELECT count(*) FROM write_events WHERE kind='reissue_credential' AND actor=?1",
                [LOCAL_OPERATOR],
                |r| r.get(0),
            )
            .unwrap(),
        )
    }

    #[test]
    fn restore_reissues_only_what_was_lost_and_keeps_what_works() {
        let mut f = initialized();
        let mcp_before = memory::stored(&f.profile, "mcp").unwrap();
        let lost_owner = memory::stored(&f.profile, "owner").unwrap();
        memory::lose(&f.profile, "owner");
        let (epoch, events) = epoch_and_events(&f.store);

        let restored = restore_access(&mut f.store, &f.profile).unwrap();

        assert_eq!(
            restored,
            Restored {
                initialized: false,
                reissued: vec!["owner".into()],
                kept: vec!["mcp".into()]
            }
        );
        assert!(works(&f, "owner") && works(&f, "mcp"));
        assert_eq!(memory::stored(&f.profile, "mcp").unwrap(), mcp_before);
        assert!(
            f.store.authenticate(&lost_owner).is_err(),
            "a lost credential never works again"
        );
        assert_eq!(epoch_and_events(&f.store), (epoch + 1, events + 1));
        // Nothing left to repair: a second run changes nothing.
        let again = restore_access(&mut f.store, &f.profile).unwrap();
        assert!(again.reissued.is_empty() && again.kept == ["mcp", "owner"]);
    }

    #[test]
    fn restore_replaces_a_stored_credential_the_profile_no_longer_accepts() {
        let mut f = initialized();
        memory::put(&f.profile, "mcp", &format!("cap:{}", "0".repeat(64)));
        let restored = restore_access(&mut f.store, &f.profile).unwrap();
        assert_eq!(restored.reissued, ["mcp"]);
        assert!(works(&f, "mcp"));
    }

    #[test]
    fn a_store_that_cannot_be_read_stops_the_repair_before_any_change() {
        let mut f = initialized();
        memory::lose(&f.profile, "owner");
        memory::break_with(&f.profile, "mcp", Some(Fault::Refused));
        let before = epoch_and_events(&f.store);

        let error = restore_access(&mut f.store, &f.profile).unwrap_err();

        assert_eq!(error.code, "AUTH_REQUIRED");
        assert!(error.message.contains("Unlock"), "{}", error.message);
        assert_eq!(
            memory::stored(&f.profile, "owner"),
            None,
            "nothing reissued"
        );
        assert_eq!(epoch_and_events(&f.store), before);
        memory::break_with(&f.profile, "mcp", None);
        assert!(works(&f, "mcp"), "a locked credential is not a lost one");
    }

    #[test]
    fn restore_finishes_an_initialization_that_stopped_before_its_principals() {
        let folder = tempfile::tempdir().unwrap();
        let profile = Profile::new(folder.path().join("profile")).unwrap();
        let mut store = Store::open(profile.clone()).unwrap();
        let restored = restore_access(&mut store, &profile).unwrap();
        assert!(restored.initialized);
        let token = memory::stored(&profile, "owner").unwrap();
        assert!(store
            .authenticate(&token)
            .unwrap()
            .permissions
            .contains("admin"));
    }

    #[test]
    fn rotation_ends_the_old_credential_and_keeps_scopes_and_permissions() {
        let mut f = initialized();
        let old = memory::stored(&f.profile, "mcp").unwrap();
        let before = f.store.authenticate(&old).unwrap();

        rotate_credential(&mut f.store, &f.profile, "mcp").unwrap();

        let new = memory::stored(&f.profile, "mcp").unwrap();
        assert_ne!(new, old);
        assert!(f.store.authenticate(&old).is_err());
        let after = f.store.authenticate(&new).unwrap();
        assert_eq!(
            (after.permissions, after.read_scopes, after.write_scopes),
            (before.permissions, before.read_scopes, before.write_scopes)
        );
    }

    #[test]
    fn a_revoked_or_unknown_principal_is_never_given_a_credential() {
        let mut f = initialized();
        f.store
            .conn
            .execute("UPDATE principals SET enabled=0 WHERE actor='mcp'", [])
            .unwrap();
        memory::lose(&f.profile, "mcp");
        for actor in ["mcp", "nobody"] {
            let error = rotate_credential(&mut f.store, &f.profile, actor).unwrap_err();
            assert_eq!(error.code, "NOT_FOUND", "{actor}");
        }
        let restored = restore_access(&mut f.store, &f.profile).unwrap();
        assert_eq!(
            (restored.reissued, restored.kept),
            (vec![], vec!["owner".to_owned()])
        );
        assert_eq!(memory::stored(&f.profile, "mcp"), None);
    }

    #[test]
    fn a_store_that_refuses_the_write_leaves_the_database_as_it_was() {
        let mut f = initialized();
        let old = memory::stored(&f.profile, "owner").unwrap();
        let before = epoch_and_events(&f.store);
        memory::break_with(&f.profile, "owner", Some(Fault::Refused));

        let error = rotate_credential(&mut f.store, &f.profile, "owner").unwrap_err();

        assert_eq!(error.code, "SECRET_STORE_UNAVAILABLE");
        memory::break_with(&f.profile, "owner", None);
        assert_eq!(memory::stored(&f.profile, "owner").unwrap(), old);
        assert!(works(&f, "owner"));
        assert_eq!(epoch_and_events(&f.store), before);
    }

    #[test]
    fn each_secret_store_problem_says_what_to_do_next() {
        let folder = tempfile::tempdir().unwrap();
        let profile = Profile::new(folder.path().join("profile")).unwrap();
        let never_set_up = OsSecrets::get(&profile, "owner").unwrap_err();
        assert!(
            never_set_up.message.contains("huda init"),
            "{}",
            never_set_up.message
        );
        drop(Store::open(profile.clone()).unwrap());
        let lost = OsSecrets::get(&profile, "owner").unwrap_err();
        assert!(
            lost.message.contains("huda credentials restore"),
            "{}",
            lost.message
        );
        assert!(!lost.message.contains("Unlock"), "{}", lost.message);

        let locked = store_error(Fault::Refused, false);
        assert!(locked.message.contains("Unlock") && !locked.message.contains("init"));
        let headless = store_error(Fault::Unavailable, false);
        assert!(
            headless.message.contains("Secret Service"),
            "{}",
            headless.message
        );
        for fault in [
            Fault::Refused,
            Fault::Failed,
            Fault::Unavailable,
            Fault::NotBuilt,
        ] {
            assert_eq!(store_error(fault, false).code, "AUTH_REQUIRED");
            assert_eq!(store_error(fault, true).code, "SECRET_STORE_UNAVAILABLE");
            assert_eq!(store_error(fault, true).status(), 503);
        }
    }

    #[cfg(feature = "os-secrets")]
    #[test]
    fn keyring_errors_map_to_what_the_person_can_do() {
        let locked = keyring::v1::Error::NoStorageAccess(Box::new(std::io::Error::other("locked")));
        assert_eq!(fault_of(&locked), Fault::Refused);
        assert_eq!(
            fault_of(&keyring::v1::Error::NoDefaultStore),
            Fault::Unavailable
        );
        let failed = keyring::v1::Error::PlatformFailure(Box::new(std::io::Error::other("bus")));
        assert_eq!(fault_of(&failed), Fault::Failed);
    }
}
