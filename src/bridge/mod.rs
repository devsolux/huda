//! Approved outbound MCP reads. This adapter captures data; admission owns meaning and writes.
//! The initial process backend requires a qualified Linux bubblewrap installation. No shell,
//! unsandboxed fallback, remote endpoint, credential forwarding, or model-selected commands.
use crate::acquisition::admission::{Capture, ExtractedData, SourceBatch};
use crate::acquisition::expand::JobView;
use crate::api::App;
use crate::error::{require, Error, Result};
use crate::relation::*;
use crate::storage::{audit, tick, Store};
use rusqlite::{params, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApprovedRead {
    Tool { name: String, arguments: Value },
    Resource { uri: String },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BridgeConfig {
    pub bridge_id: String,
    pub source_key: String,
    pub scope: String,
    pub enabled: bool,
    pub executable: String,
    pub executable_sha256: String,
    pub arguments: Vec<String>,
    /// Optional exact regular input file, mounted read-only as /input/source. No directory grants.
    pub input_file: Option<String>,
    pub read: ApprovedRead,
    pub semantic_text: bool,
    pub ttl_seconds: u64,
    pub license: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigureBridgeRequest {
    pub config: BridgeConfig,
    pub expected_version: u64,
    pub expected_policy_epoch: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureBridgeRequest {
    pub bridge_id: String,
    pub expected_generation: u64,
    pub expected_policy_epoch: u64,
    pub idempotency_key: String,
}
impl BridgeConfig {
    fn validate(&self, profile: &Path) -> Result<()> {
        identifier(&self.bridge_id)?;
        identifier(&self.source_key)?;
        identifier(&self.scope)?;
        require(
            (60..=604800).contains(&self.ttl_seconds),
            "INVALID_INPUT",
            "Bridge TTL exceeds allowed bounds",
        )?;
        text(&self.license, 256)?;
        require(
            self.executable_sha256.len() == 64
                && self
                    .executable_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit()),
            "INVALID_INPUT",
            "Bridge executable requires a verified SHA-256",
        )?;
        require(
            self.arguments.len() <= 16,
            "LIMIT_EXCEEDED",
            "Too many fixed executable arguments",
        )?;
        for argument in &self.arguments {
            text(argument, 1024)?;
        }
        approved_file(&self.executable, profile, 128 * 1024 * 1024)?;
        if let Some(input) = &self.input_file {
            approved_file(input, profile, crate::MAX_SOURCE_BYTES as u64)?;
        }
        match &self.read {
            ApprovedRead::Tool { name, arguments } => {
                identifier(name)?;
                require(
                    arguments.is_object() && canonical_bytes(arguments)?.len() <= 8192,
                    "INVALID_INPUT",
                    "Approved tool arguments must be a bounded object",
                )?;
            }
            ApprovedRead::Resource { uri } => {
                text(uri, 2048)?;
                require(
                    !uri.contains('\n') && !uri.contains('\r'),
                    "INVALID_INPUT",
                    "Invalid resource identifier",
                )?;
            }
        }
        Ok(())
    }
}
fn approved_file(input: &str, profile: &Path, max: u64) -> Result<PathBuf> {
    let path = Path::new(input);
    require(
        path.is_absolute()
            && !path
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir)),
        "INVALID_INPUT",
        "Bridge paths must be absolute, explicit regular files",
    )?;
    let metadata = std::fs::symlink_metadata(path)?;
    require(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= max,
        "POLICY_DENIED",
        "Bridge input/executable is not a qualified regular file",
    )?;
    let canonical = std::fs::canonicalize(path)?;
    require(
        !canonical.starts_with(profile),
        "POLICY_DENIED",
        "Huda profile files cannot be exposed to a Bridge process",
    )?;
    Ok(canonical)
}
impl Store {
    pub fn configure_bridge(
        &mut self,
        caller: &Caller,
        request: ConfigureBridgeRequest,
    ) -> Result<()> {
        caller.require("policy")?;
        caller.require("bridge")?;
        request.config.validate(self.profile.directory())?;
        require(
            caller.write(&request.config.scope),
            "PERMISSION_DENIED",
            "Bridge scope is not writable",
        )?;
        self.source_generation(caller, &request.config.source_key)?;
        let tx = self.begin(caller, request.expected_policy_epoch)?;
        let prior: Option<(String, String, u64)> = tx
            .query_row(
                "SELECT owner,scope,version FROM bridge_sources WHERE bridge_id=?1",
                [&request.config.bridge_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((owner, scope, version)) = prior {
            require(
                owner == caller.actor
                    && scope == request.config.scope
                    && version == request.expected_version,
                "VERSION_CONFLICT",
                "Bridge ownership or version changed",
            )?;
        } else {
            require(
                request.expected_version == 0,
                "VERSION_CONFLICT",
                "New Bridge configuration requires expected_version=0",
            )?;
        }
        let source_scope: String = tx.query_row(
            "SELECT scope FROM sources WHERE source_key=?1",
            [&request.config.source_key],
            |r| r.get(0),
        )?;
        require(
            source_scope == request.config.scope,
            "POLICY_DENIED",
            "Bridge and admitted source scopes must match",
        )?;
        tx.execute("INSERT INTO bridge_sources(bridge_id,owner,scope,config_json,version) VALUES(?1,?2,?3,?4,1) ON CONFLICT(bridge_id) DO UPDATE SET config_json=excluded.config_json,version=bridge_sources.version+1",params![request.config.bridge_id,caller.actor,request.config.scope,serde_json::to_string(&request.config)?])?;
        let t = tick(&tx)?;
        tx.execute(
            "UPDATE system_meta SET policy_epoch=policy_epoch+1 WHERE singleton=1",
            [],
        )?;
        audit(
            &tx,
            t,
            caller,
            "configure_bridge",
            &request.config.bridge_id,
        )?;
        caller.check()?;
        tx.commit()?;
        Ok(())
    }
    pub fn bridge_status(&self, caller: &Caller) -> Result<Value> {
        caller.require("bridge")?;
        let mut statement=self.conn.prepare("SELECT bridge_id,scope,config_json,version FROM bridge_sources WHERE owner=?1 ORDER BY bridge_id LIMIT 65")?;
        let rows = statement
            .query_map([&caller.actor], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u64>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let truncated = rows.len() > 64;
        let mut values = Vec::new();
        for (id, scope, config, version) in rows.into_iter().take(64) {
            if caller.read(&scope) {
                let config: BridgeConfig = serde_json::from_str(&config)?;
                values.push(json!({"bridge_id":id,"version":version,"enabled":config.enabled,"state":if !config.enabled{"disabled"}else if cfg!(target_os="linux") && Path::new("/usr/bin/bwrap").is_file(){"configured_unqualified"}else{"unsupported_platform"}}));
            }
        }
        Ok(json!({"sources":values,"truncated":truncated,"remote_http_supported":false}))
    }
    fn bridge_config(&self, caller: &Caller, id: &str, epoch: u64) -> Result<BridgeConfig> {
        self.check_policy(caller, epoch)?;
        caller.require("bridge")?;
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT config_json FROM bridge_sources WHERE bridge_id=?1 AND owner=?2",
                params![id, caller.actor],
                |r| r.get(0),
            )
            .optional()?;
        let config: BridgeConfig = serde_json::from_str(&raw.ok_or_else(Error::not_found)?)?;
        require(
            config.enabled && caller.write(&config.scope),
            "POLICY_DENIED",
            "Bridge is disabled or outside caller scope",
        )?;
        // Offline is conservative: it also blocks explicit local capture, never grants egress.
        require(
            !self.epochs()?.offline,
            "OFFLINE",
            "Bridge capture is disabled by offline policy",
        )?;
        Ok(config)
    }
}
impl App {
    pub fn capture_bridge(
        &self,
        caller: &Caller,
        request: CaptureBridgeRequest,
    ) -> Result<JobView> {
        caller.require("ingest")?;
        text(&request.idempotency_key, 128)?;
        let config = self.store()?.bridge_config(
            caller,
            &request.bridge_id,
            request.expected_policy_epoch,
        )?;
        let goal = json!({"bridge_id":config.bridge_id,"source_key":config.source_key});
        let scope = config.scope.clone();
        let epoch = request.expected_policy_epoch;
        self.spawn_job(
            caller,
            "mcp_capture",
            &scope,
            &goal,
            epoch,
            move |app, caller, id| {
                app.store()?
                    .bridge_config(&caller, &config.bridge_id, epoch)?;
                let profile = app.store()?.profile.directory().to_path_buf();
                config.validate(&profile)?;
                let result = capture_process(&app, &caller, &config, epoch)?;
                app.store()?.check_policy(&caller, epoch)?;
                let (data, semantic) = if config.semantic_text {
                    let content = source_text(&result)?;
                    let (entities, types) = {
                        let mut store = app.store()?;
                        (store.visible_entities(&caller)?, store.list_types(&caller)?)
                    };
                    (
                        crate::acquisition::text::extract(
                            app.model.as_ref(),
                            &config.source_key,
                            &content,
                            &entities,
                            &types,
                        )?,
                        true,
                    )
                } else {
                    let value = if let Some(value) = result.get("structuredContent") {
                        value.clone()
                    } else {
                        serde_json::from_str(&source_text(&result)?).map_err(|_| {
                            Error::new(
                                "UNSUPPORTED_SCHEMA",
                                "Bridge resource must contain typed relation JSON",
                            )
                        })?
                    };
                    (
                        serde_json::from_value::<ExtractedData>(value).map_err(|_| {
                            Error::new("UNSUPPORTED_SCHEMA", "Bridge output is not ExtractedData")
                        })?,
                        false,
                    )
                };
                let revision = stable_id("capture", &result)?;
                let mut capture = Capture::local(revision, result)?;
                capture.source_uri = Some(format!("mcp-capture:{}", config.bridge_id));
                capture.external_id = Some(config.bridge_id.clone());
                capture.license = Some(config.license.clone());
                capture.expires_at = Some(capture.retrieved_at + config.ttl_seconds * 1000);
                capture.attribution = Some(format!(
                    "Approved MCP source {}; executable SHA-256 {}",
                    config.bridge_id, config.executable_sha256
                ));
                let batch = SourceBatch {
                    source_key: config.source_key,
                    expected_generation: request.expected_generation,
                    scope_key: "approved-read".into(),
                    complete: false,
                    extractor_digest: "mcp-capture-v1".into(),
                    origin_kind: "mcp_capture".into(),
                    semantic,
                    data,
                    capture,
                    request_key: format!("bridge:{}", request.idempotency_key),
                    policy_epoch: epoch,
                    job_id: Some(id),
                    provider_properties: Vec::new(),
                    raw_proposals: Vec::new(),
                };
                let imported = app.store()?.publish_source(&caller, batch)?;
                Ok(json!({"stop_reason":"captured","import":imported}))
            },
        )
    }
}
fn source_text(result: &Value) -> Result<String> {
    let contents = result
        .get("contents")
        .or_else(|| result.get("content"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::new("UNSUPPORTED_SCHEMA", "MCP result has no supported content"))?;
    require(
        contents.len() == 1,
        "UNSUPPORTED_SCHEMA",
        "This Bridge mapping requires exactly one text resource",
    )?;
    let value = contents[0]["text"]
        .as_str()
        .ok_or_else(|| Error::new("UNSUPPORTED_SCHEMA", "Bridge content is not text"))?;
    text(value, 262_144)?;
    Ok(value.into())
}
#[cfg(not(target_os = "linux"))]
fn capture_process(
    _app: &App,
    _caller: &Caller,
    _config: &BridgeConfig,
    _epoch: u64,
) -> Result<Value> {
    Err(Error::new("UNSUPPORTED_PLATFORM","A qualified native subprocess sandbox is required for Bridge capture; no unsandboxed fallback"))
}
#[cfg(target_os = "linux")]
fn capture_process(app: &App, caller: &Caller, config: &BridgeConfig, epoch: u64) -> Result<Value> {
    use std::io::{BufReader, Read};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    let wrapper = Path::new("/usr/bin/bwrap");
    let metadata = std::fs::symlink_metadata(wrapper).map_err(|_| {
        Error::new(
            "ADAPTER_UNAVAILABLE",
            "Install and qualify native bubblewrap explicitly before enabling this Bridge",
        )
    })?;
    require(
        metadata.is_file() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0,
        "POLICY_DENIED",
        "Subprocess sandbox has unsafe ownership or permissions",
    )?;
    let mut executable = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&config.executable)?;
    let metadata = executable.metadata()?;
    require(
        metadata.is_file() && metadata.len() <= 128 * 1024 * 1024,
        "POLICY_DENIED",
        "Bridge executable size/type changed",
    )?;
    let mut bytes = Vec::new();
    executable
        .by_ref()
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    require(
        bytes.starts_with(b"\x7fELF")
            && sha256_hex(&bytes).eq_ignore_ascii_case(&config.executable_sha256),
        "POLICY_DENIED",
        "Bridge binary digest changed or input is not a native ELF executable",
    )?;
    drop(bytes);
    // The explicit executable can still be replaced by the same OS user between checking
    // and exec. Same-user hostile process isolation is not a qualified security claim.
    let mut command = Command::new(wrapper);
    command.env_clear().args([
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--ro-bind",
        "/usr",
        "/usr",
    ]);
    for directory in ["/lib", "/lib64"] {
        if Path::new(directory).exists() {
            command.args(["--ro-bind", directory, directory]);
        }
    }
    command
        .args([
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/app",
            "--ro-bind",
        ])
        .arg(&config.executable)
        .arg("/app/provider");
    if let Some(file) = &config.input_file {
        command
            .args(["--dir", "/input", "--ro-bind"])
            .arg(file)
            .arg("/input/source");
    }
    command
        .args([
            "--chdir",
            "/tmp",
            "--setenv",
            "HOME",
            "/tmp",
            "--setenv",
            "LANG",
            "C.UTF-8",
            "--",
            "/app/provider",
        ])
        .args(&config.arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: child hook only calls async-signal-safe setrlimit; no allocation or locks.
    unsafe {
        command.pre_exec(|| {
            for (resource, limit) in [
                (libc::RLIMIT_CPU, 15),
                (libc::RLIMIT_AS, 1024 * 1024 * 1024),
                (libc::RLIMIT_FSIZE, 16 * 1024 * 1024),
                (libc::RLIMIT_NOFILE, 64),
                (libc::RLIMIT_NPROC, 64),
            ] {
                let value = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: limit,
                };
                if libc::setrlimit(resource, &value) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    app.store()?
        .bridge_config(caller, &config.bridge_id, epoch)?;
    struct Process(std::process::Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut process = Process(command.spawn().map_err(|_| {
        Error::new(
            "ADAPTER_UNAVAILABLE",
            "Sandboxed MCP process could not start",
        )
    })?);
    let mut input = process
        .0
        .stdin
        .take()
        .ok_or_else(|| Error::new("SOURCE_FAILED", "MCP process has no stdin"))?;
    let output = process
        .0
        .stdout
        .take()
        .ok_or_else(|| Error::new("SOURCE_FAILED", "MCP process has no stdout"))?;
    let (sender, receiver) = mpsc::sync_channel(2);
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(output);
        for _ in 0..4 {
            match crate::runtime::framing::read_frame(&mut reader, crate::MAX_MESSAGE_BYTES) {
                Ok(Some(bytes)) => {
                    if sender
                        .send(serde_json::from_slice::<Value>(&bytes).map_err(Error::from))
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = sender.send(Err(error));
                    break;
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let receive = |expected: &str| -> Result<Value> {
        loop {
            caller.check()?;
            app.store()?.check_policy(caller, epoch)?;
            require(
                Instant::now() < deadline,
                "DEADLINE_EXCEEDED",
                "MCP capture deadline exceeded",
            )?;
            match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(value) => {
                    let value = value?;
                    require(
                        value["jsonrpc"] == "2.0"
                            && value["id"] == expected
                            && value.get("method").is_none(),
                        "UNSUPPORTED_PROTOCOL",
                        "Bridge refused a callback, mismatched ID, or unsolicited request",
                    )?;
                    require(
                        value.get("error").is_none(),
                        "SOURCE_FAILED",
                        "Approved MCP read returned a protocol error",
                    )?;
                    let result = value.get("result").ok_or_else(|| {
                        Error::new("UNSUPPORTED_PROTOCOL", "MCP result is absent")
                    })?;
                    require(
                        result["resultType"] == "complete",
                        "UNSUPPORTED_PROTOCOL",
                        "Only complete modern MCP results are supported",
                    )?;
                    require(
                        result["isError"] != true,
                        "SOURCE_FAILED",
                        "Approved MCP tool returned an error",
                    )?;
                    return Ok(result.clone());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => {
                    return Err(Error::new(
                        "SOURCE_FAILED",
                        "MCP process closed before completing the read",
                    ))
                }
            }
        }
    };
    let result = (|| {
        let meta = crate::mcp::request_meta();
        crate::runtime::framing::write_frame(
            &mut input,
            &json!({"jsonrpc":"2.0","id":"discover","method":"server/discover","params":{"_meta":meta}}),
            crate::MAX_MESSAGE_BYTES,
        )?;
        let discovery = receive("discover")?;
        require(
            discovery["supportedVersions"]
                .as_array()
                .is_some_and(|versions| versions.iter().any(|v| v == crate::MCP_VERSION)),
            "UNSUPPORTED_PROTOCOL",
            "Bridge server does not support the pinned MCP version",
        )?;
        require(
            !discovery
                .pointer("/_meta/io.modelcontextprotocol~1serverInfo/name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case("huda")),
            "POLICY_DENIED",
            "Huda-to-Huda Bridge cycles are forbidden",
        )?;
        app.store()?
            .bridge_config(caller, &config.bridge_id, epoch)?;
        let (method, mut params) = match &config.read {
            ApprovedRead::Tool { name, arguments } => {
                ("tools/call", json!({"name":name,"arguments":arguments}))
            }
            ApprovedRead::Resource { uri } => ("resources/read", json!({"uri":uri})),
        };
        params["_meta"] = crate::mcp::request_meta();
        crate::runtime::framing::write_frame(
            &mut input,
            &json!({"jsonrpc":"2.0","id":"read","method":method,"params":params}),
            crate::MAX_MESSAGE_BYTES,
        )?;
        receive("read")
    })();
    drop(receiver);
    drop(input);
    drop(process);
    let _ = reader.join();
    result
}
