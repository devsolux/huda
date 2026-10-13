//! The only authoritative connection is retained by Store under the application mutex.
use crate::error::{require, Error, Result};
use crate::relation::{now_ms, Caller};
use crate::runtime::{profile::Lease, Profile};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use std::path::Path;

pub const APPLICATION_ID: i32 = 0x48554441; // "HUDA"
pub const SCHEMA_VERSION: i32 = 1;
pub const SCHEMA: &str = include_str!("../../migrations/0001_authority.sql");

pub struct Store {
    pub(crate) conn: Connection,
    pub(crate) profile: Profile,
    _lease: Lease,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Epochs {
    pub policy: u64,
    pub registry: u64,
    pub offline: bool,
}
impl Store {
    pub fn open(profile: Profile) -> Result<Self> {
        let lease = profile.acquire()?;
        let is_new = profile.reserve_database()?;
        if !is_new {
            let reader=open_reader_path(&profile.database()).map_err(|error|if error.code=="DB_INCOMPATIBLE" {error}else{Error::new("READ_ONLY_RECOVERY","Existing database could not be read safely; no repair or wipe was attempted")})?;
            integrity(&reader)?;
        }
        let conn = Connection::open_with_flags(
            profile.database(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(3))?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL; PRAGMA mmap_size=0;")?;
        // Schema/history are never inferred from a filename or repaired by dropping tables.
        if is_new {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            if let Err(e) = conn
                .execute_batch(SCHEMA)
                .and_then(|_| conn.pragma_update(None, "application_id", APPLICATION_ID))
                .and_then(|_| conn.pragma_update(None, "user_version", SCHEMA_VERSION))
                .and_then(|_| conn.execute_batch("COMMIT;"))
            {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(e.into());
            }
        }
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH, 16_777_216)?;
        conn.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH, 131_072)?;
        let mut store = Self {
            conn,
            profile,
            _lease: lease,
        };
        store.bootstrap_types()?;
        store.recover_jobs()?;
        Ok(store)
    }
    pub fn open_reader(&self) -> Result<Connection> {
        open_reader_path(&self.profile.database())
    }
    pub fn epochs(&self) -> Result<Epochs> {
        read_epochs(&self.conn)
    }
    pub(crate) fn begin(&mut self, caller: &Caller, epoch: u64) -> Result<Transaction<'_>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        recheck(&tx, caller, epoch)?;
        Ok(tx)
    }
    pub fn backup(&self, caller: &Caller, filename: &str) -> Result<String> {
        caller.require("admin")?;
        require(
            filename
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                && !filename.is_empty()
                && filename.len() <= 80,
            "INVALID_INPUT",
            "Backup name must be a simple basename",
        )?;
        let name = format!("backup-{filename}.db");
        let path = self.profile.directory().join(&name);
        require(!path.exists(), "VERSION_CONFLICT", "Backup already exists")?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&path)?;
        let mut destination =
            Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        {
            let backup = rusqlite::backup::Backup::new(&self.conn, &mut destination)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                caller.check()?;
                require(
                    std::time::Instant::now() < deadline,
                    "DEADLINE_EXCEEDED",
                    "Backup deadline exceeded; incomplete backup retained for diagnosis",
                )?;
                match backup.step(128)? {
                    rusqlite::backup::StepResult::Done => break,
                    rusqlite::backup::StepResult::Busy | rusqlite::backup::StepResult::Locked => {
                        std::thread::sleep(std::time::Duration::from_millis(25))
                    }
                    _ => {}
                }
            }
        }
        integrity(&destination)?;
        drop(destination);
        file.sync_all()?;
        Ok(name)
    }
    fn recover_jobs(&mut self) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE jobs SET state='failed',version=version+1,updated_at=?1,report_json=json_set(report_json,'$.stop_reason','core_restarted','$.publication_may_have_committed',json('true')) WHERE state IN ('queued','running')",rusqlite::params![now_ms()])?;
        tx.commit()?;
        Ok(())
    }
}
pub(crate) fn open_reader_path(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.execute_batch("PRAGMA query_only=ON; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=3000; PRAGMA mmap_size=0;")?;
    let application: i32 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    require(
        application == APPLICATION_ID && version == SCHEMA_VERSION,
        "DB_INCOMPATIBLE",
        "Database identity/version is incompatible; it was not modified",
    )?;
    Ok(conn)
}
pub(crate) fn integrity(conn: &Connection) -> Result<()> {
    let check: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    let foreign_keys: bool = conn.prepare("PRAGMA foreign_key_check")?.exists([])?;
    require(
        check == "ok" && !foreign_keys,
        "READ_ONLY_RECOVERY",
        "Database integrity check failed; no automatic repair or deletion is allowed",
    )
}
pub(crate) fn read_epochs(conn: &Connection) -> Result<Epochs> {
    Ok(conn.query_row(
        "SELECT policy_epoch,registry_epoch,offline FROM system_meta WHERE singleton=1",
        [],
        |r| {
            Ok(Epochs {
                policy: r.get(0)?,
                registry: r.get(1)?,
                offline: r.get(2)?,
            })
        },
    )?)
}
pub(crate) fn recheck(conn: &Connection, caller: &Caller, epoch: u64) -> Result<()> {
    caller.check()?;
    let current = read_epochs(conn)?;
    require(
        current.policy == epoch,
        "POLICY_CHANGED",
        "Policy changed; start a new operation",
    )?;
    let enabled: bool = conn
        .query_row(
            "SELECT enabled FROM principals WHERE actor=?1",
            [&caller.actor],
            |r| r.get(0),
        )
        .unwrap_or(false);
    require(enabled, "PERMISSION_DENIED", "Caller has been revoked")
}
pub(crate) fn tick(conn: &Connection) -> Result<u64> {
    conn.execute(
        "UPDATE system_meta SET tx_tick=tx_tick+1 WHERE singleton=1",
        [],
    )?;
    Ok(conn.query_row(
        "SELECT tx_tick FROM system_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?)
}
pub(crate) fn audit(
    conn: &Connection,
    tick: u64,
    caller: &Caller,
    kind: &str,
    target: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO write_events(event_id,tx_tick,actor,kind,target_id) VALUES(?1,?2,?3,?4,?5)",
        rusqlite::params![
            crate::relation::stable_id("op", &(tick, &caller.actor, kind, target))?,
            tick,
            caller.actor,
            kind,
            target
        ],
    )?;
    Ok(())
}
