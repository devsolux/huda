//! Request authentication is independent of model/context content.
use crate::error::{require, Error, Result};
use crate::relation::{identifier, random_id, Caller};
use crate::storage::{audit, tick, Store};
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
    pub fn add_principal(
        &mut self,
        caller: &Caller,
        request: PrincipalRequest,
        token: &str,
    ) -> Result<()> {
        caller.require("admin")?;
        identifier(&request.actor)?;
        require(
            !request.read_scopes.is_empty()
                && request.read_scopes.len() <= 32
                && request.write_scopes.is_subset(&request.read_scopes),
            "INVALID_INPUT",
            "Invalid scoped capability",
        )?;
        for scope in &request.read_scopes {
            identifier(scope)?;
        }
        for permission in &request.permissions {
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
}

/// Secrets never enter SQLite, config JSON, command-line arguments, or logs.
pub struct OsSecrets;
impl OsSecrets {
    #[cfg(feature = "os-secrets")]
    fn entry(profile: &super::Profile, actor: &str) -> Result<keyring::v1::Entry> {
        keyring::v1::Entry::new(&format!("huda.{}", profile.fingerprint()), actor)
            .map_err(|_| Error::new("AUTH_REQUIRED", "OS secret store is unavailable"))
    }
    pub fn get(profile: &super::Profile, actor: &str) -> Result<String> {
        #[cfg(feature = "os-secrets")]
        {
            Self::entry(profile, actor)?.get_password().map_err(|_| {
                Error::new(
                    "AUTH_REQUIRED",
                    "Unlock the OS secret store and initialize Huda credentials",
                )
            })
        }
        #[cfg(not(feature = "os-secrets"))]
        {
            let _ = (profile, actor);
            Err(Error::new(
                "AUTH_REQUIRED",
                "Build with os-secrets to use native clients",
            ))
        }
    }
    pub fn create(profile: &super::Profile, actor: &str) -> Result<String> {
        let token = random_id("cap")?;
        #[cfg(feature = "os-secrets")]
        {
            let entry = Self::entry(profile, actor)?;
            // Never overwrite an existing credential on a retry or database conflict.
            match entry.get_password() {
                Ok(existing) => return Ok(existing),
                Err(keyring::v1::Error::NoEntry) => {}
                Err(_) => {
                    return Err(Error::new(
                        "AUTH_REQUIRED",
                        "OS secret store is locked or unavailable",
                    ))
                }
            }
            entry.set_password(&token).map_err(|_| {
                Error::new(
                    "AUTH_REQUIRED",
                    "OS secret store could not save the credential",
                )
            })?;
            Ok(token)
        }
        #[cfg(not(feature = "os-secrets"))]
        {
            let _ = (profile, actor, token);
            Err(Error::new(
                "AUTH_REQUIRED",
                "Build with os-secrets to initialize native credentials",
            ))
        }
    }
}
