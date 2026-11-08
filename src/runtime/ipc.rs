//! A private, peer-authorized transport around the same App. No client opens SQLite.
use super::{auth::OsSecrets, framing, Profile};
use crate::api::App;
use crate::error::{require, Error, Result};
use crate::relation::{identifier, now_ms, random_id};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::BufReader;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex, MutexGuard, PoisonError,
};
use std::time::{Duration, Instant};
struct DeadlineReader<'a> {
    stream: &'a mut Stream,
    deadline: Instant,
}
impl std::io::Read for DeadlineReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "IPC frame deadline exceeded")
            })?;
        self.stream.set_read_timeout(Some(remaining))?;
        std::io::Read::read(self.stream, bytes)
    }
}
use super::transport::{Listener, Stream};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    ipc_version: u32,
    client_version: String,
    correlation_id: String,
    deadline_ms: u64,
    token: String,
    operation: String,
    input: Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    ipc_version: u32,
    server_version: String,
    correlation_id: String,
    result: Option<Value>,
    error: Option<Error>,
}
/// Whether the OS secret store holds a client's credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credential {
    Stored,
    Missing,
}
/// The profile fingerprint and actor a remembered credential belongs to, and the credential.
type Remembered = Option<((String, String), String)>;

#[derive(Clone)]
pub struct Client {
    pub profile: Profile,
    pub actor: String,
    pub start_on_demand: bool,
    /// Read once and kept for this client and its clones, so a long-lived client (the desktop, an
    /// MCP server) asks the OS secret store once, not on every request: each read may show a
    /// system prompt. Keyed by profile and actor, so it never answers for another.
    remembered: Arc<Mutex<Remembered>>,
}
impl Client {
    pub fn new(profile: Profile, actor: impl Into<String>, start_on_demand: bool) -> Self {
        Self {
            profile,
            actor: actor.into(),
            start_on_demand,
            remembered: Arc::default(),
        }
    }
    /// Reads this client's credential without sending anything, and keeps it for the requests
    /// that follow. A missing credential and a store that refuses access need different next
    /// steps, so a caller can tell them apart before it asks the Core.
    pub fn credential(&self) -> Result<Credential> {
        identifier(&self.actor)?;
        let key = self.key();
        let mut slot = self.slot();
        if !slot.as_ref().is_some_and(|(held, _)| *held == key) {
            *slot = OsSecrets::read(&self.profile, &self.actor)?.map(|token| (key, token));
        }
        Ok(match *slot {
            Some(_) => Credential::Stored,
            None => Credential::Missing,
        })
    }
    /// Drops the remembered credential, so the next request reads the OS secret store again.
    pub fn forget_credential(&self) {
        *self.slot() = None;
    }
    pub fn invoke(&self, operation: &str, input: Value) -> Result<Value> {
        self.invoke_cancelled(operation, input, Arc::new(AtomicBool::new(false)))
    }
    pub fn invoke_cancelled(
        &self,
        operation: &str,
        input: Value,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Value> {
        identifier(&self.actor)?;
        check_cancelled(&cancelled)?;
        // Do not spawn a Core as a side effect of a missing/unavailable credential.
        let (token, remembered) = self.token()?;
        check_cancelled(&cancelled)?;
        let mut request = serde_json::to_value(Envelope {
            ipc_version: crate::IPC_VERSION,
            client_version: crate::VERSION.into(),
            correlation_id: String::new(),
            deadline_ms: 0,
            token,
            operation: operation.into(),
            input,
        })?;
        let mut result = self.send(&mut request, &cancelled);
        // The Core checks a credential before it runs anything, so a request it refused ran
        // nothing. A remembered credential may have been replaced since it was read (restored or
        // rotated), so it is read again, and the request is sent once more only if it changed.
        if remembered && matches!(&result, Err(error) if error.code == "AUTH_REQUIRED") {
            self.forget_credential();
            let (fresh, _) = self.token()?;
            if request["token"] != fresh.as_str() {
                request["token"] = Value::String(fresh);
                result = self.send(&mut request, &cancelled);
            }
        }
        match result {
            Err(error) if error.code == "AUTH_REQUIRED" => Err(Error::new(
                "AUTH_REQUIRED",
                &format!("This profile does not accept the {} credential in the OS secret store. If its access was revoked, ask the profile owner; otherwise restore access with huda credentials restore, or with Restore Access in the desktop", self.actor),
            )),
            other => other,
        }
    }
    /// The credential to send, and whether it was remembered from an earlier read.
    fn token(&self) -> Result<(String, bool)> {
        let key = self.key();
        let mut slot = self.slot();
        if let Some((held, token)) = slot.as_ref() {
            if *held == key {
                return Ok((token.clone(), true));
            }
        }
        let token = OsSecrets::get(&self.profile, &self.actor)?;
        *slot = Some((key, token.clone()));
        Ok((token, false))
    }
    fn key(&self) -> (String, String) {
        (self.profile.fingerprint(), self.actor.clone())
    }
    fn slot(&self) -> MutexGuard<'_, Remembered> {
        self.remembered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
    /// One exchange with the Core, under a new correlation ID and deadline.
    fn send(&self, request: &mut Value, cancelled: &Arc<AtomicBool>) -> Result<Value> {
        let mut stream = self.connect(cancelled)?;
        stream.verify_peer()?;
        check_cancelled(cancelled)?;
        let correlation_id = random_id("request")?;
        request["correlation_id"] = Value::String(correlation_id.clone());
        request["deadline_ms"] = json!(now_ms() + 120_000);
        stream.set_read_timeout(Some(Duration::from_secs(120)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        let monitor = stream.try_clone()?;
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let signal = Arc::clone(cancelled);
        let watcher = std::thread::spawn(move || {
            while !finished.load(Ordering::Acquire) {
                if signal.load(Ordering::Acquire) {
                    monitor.shutdown();
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let result = (|| {
            framing::write_frame(&mut stream, request, crate::MAX_MESSAGE_BYTES)?;
            let frame = framing::read_frame(
                &mut BufReader::new(DeadlineReader {
                    stream: &mut stream,
                    deadline: Instant::now() + Duration::from_secs(120),
                }),
                crate::MAX_MESSAGE_BYTES,
            )?
            .ok_or_else(|| Error::new("CORE_UNAVAILABLE", "Core closed the request"))?;
            let response: Response = serde_json::from_slice(&frame)?;
            require(
                response.ipc_version == crate::IPC_VERSION
                    && response.server_version == crate::VERSION,
                "DB_INCOMPATIBLE",
                "Core/helper versions are incompatible",
            )?;
            // A result is only accepted for this exact request. A redacted error may carry an
            // empty correlation when the Core could not parse the envelope at all.
            let correlated = response.correlation_id == correlation_id;
            match (response.result, response.error) {
                (Some(result), None) if correlated => Ok(result),
                (None, Some(error)) if correlated || response.correlation_id.is_empty() => {
                    Err(error)
                }
                _ => Err(Error::invalid("Invalid Core response envelope")),
            }
        })();
        done.store(true, Ordering::Release);
        let _ = watcher.join();
        if cancelled.load(Ordering::Acquire) {
            return Err(Error::new(
                "CANCELLED",
                "Request was cancelled; committed transactions are not rolled back",
            ));
        }
        result
    }
    fn connect(&self, cancelled: &AtomicBool) -> Result<Stream> {
        check_cancelled(cancelled)?;
        match Stream::connect(&self.profile) {
            Ok(stream) => return Ok(stream),
            Err(error) if error.code == "CORE_UNAVAILABLE" => {}
            Err(error) => return Err(error),
        }
        require(
            self.start_on_demand,
            "CORE_UNAVAILABLE",
            "Start huda core before using this client",
        )?;
        check_cancelled(cancelled)?;
        let mut child = super::helper::command()?
            .arg("--profile")
            .arg(self.profile.directory())
            .args(["core", "--idle-seconds", "300"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let result = self.wait_until_ready(&mut child, cancelled);
        // Reap our child; never kill a shared Core just because this client closes.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        result
    }
    fn wait_until_ready(
        &self,
        child: &mut std::process::Child,
        cancelled: &AtomicBool,
    ) -> Result<Stream> {
        let mut deadline = Instant::now() + Duration::from_secs(5);
        let mut exited = false;
        loop {
            check_cancelled(cancelled)?;
            match Stream::connect(&self.profile) {
                Ok(stream) => return Ok(stream),
                Err(error) if error.code == "CORE_UNAVAILABLE" => {}
                Err(error) => return Err(error),
            }
            if !exited && child.try_wait()?.is_some() {
                // Our Core may have lost a start race to another client's; let that one listen.
                exited = true;
                deadline = deadline.min(Instant::now() + Duration::from_secs(1));
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err(Error::new(
            "CORE_UNAVAILABLE",
            if exited {
                "Core exited during startup; run huda core in the foreground for its redacted error"
            } else {
                "Core did not become ready; run huda core in the foreground for its redacted error"
            },
        ))
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
    require(
        !cancelled.load(Ordering::Acquire),
        "CANCELLED",
        "Request was cancelled before dispatch",
    )
}

pub fn serve(
    app: App,
    profile: Profile,
    idle_seconds: u64,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    serve_announced(app, profile, idle_seconds, shutdown, || {})
}
/// Like `serve`, calling `ready` once the private listener is bound, so a readiness message is
/// never shown for a Core that then fails to listen.
pub fn serve_announced(
    app: App,
    profile: Profile,
    idle_seconds: u64,
    shutdown: Arc<AtomicBool>,
    ready: impl FnOnce(),
) -> Result<()> {
    let mut listener = Listener::bind(&profile)?;
    ready();
    let clients = Arc::new(AtomicUsize::new(0));
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok(stream) => {
                if clients
                    .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                        (n < 16).then_some(n + 1)
                    })
                    .is_err()
                {
                    drop(stream);
                    continue;
                }
                let clients = clients.clone();
                let app = app.clone();
                std::thread::spawn(move || {
                    // Release the slot even if the handler panics; leaked slots would refuse
                    // every client and keep the Core from retiring when idle.
                    struct Slot(Arc<AtomicUsize>);
                    impl Drop for Slot {
                        fn drop(&mut self) {
                            self.0.fetch_sub(1, Ordering::AcqRel);
                        }
                    }
                    let _slot = Slot(clients);
                    let _ = handle(app, stream);
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(error) => return Err(error.into()),
        }
        if idle_seconds > 0
            && clients.load(Ordering::Acquire) == 0
            && app.running_jobs() == 0
            && now_ms().saturating_sub(app.last_activity()) > idle_seconds.saturating_mul(1000)
        {
            break;
        }
    }
    shutdown.store(true, Ordering::Release);
    drop(listener);
    Ok(())
}

fn handle(app: App, mut stream: Stream) -> Result<()> {
    stream.verify_peer()?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let frame = framing::read_frame(
        &mut BufReader::new(DeadlineReader {
            stream: &mut stream,
            deadline: Instant::now() + Duration::from_secs(3),
        }),
        crate::MAX_MESSAGE_BYTES,
    )?
    .ok_or_else(|| Error::invalid("IPC request is empty"))?;
    let envelope: Envelope = match serde_json::from_slice(&frame) {
        Ok(envelope) => envelope,
        Err(_) => {
            // The correlation ID is unknown when the envelope itself is malformed or carries
            // unknown (for example forged authority) fields. Answer once with an empty
            // correlation so the client reports a request error, not a closed Core.
            let error = Error::invalid("Malformed or unsupported IPC envelope");
            return framing::write_frame(
                &mut stream,
                &json!({"ipc_version":crate::IPC_VERSION,"server_version":crate::VERSION,"correlation_id":"","result":null,"error":error}),
                crate::MAX_MESSAGE_BYTES,
            );
        }
    };
    let id = envelope.correlation_id.clone();
    let result = (|| {
        identifier(&envelope.correlation_id)?;
        require(
            envelope.ipc_version == crate::IPC_VERSION && envelope.client_version == crate::VERSION,
            "DB_INCOMPATIBLE",
            "Core/helper versions are incompatible",
        )?;
        let caller = app.authenticate(&envelope.token, envelope.deadline_ms)?;
        let monitor = stream.try_clone()?;
        let flag = caller.cancelled.clone();
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let watcher = std::thread::spawn(move || {
            while !finished.load(Ordering::Acquire) {
                if monitor.disconnected_or_extra_data() {
                    flag.store(true, Ordering::Release);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let result = app.dispatch(&caller, &envelope.operation, envelope.input);
        done.store(true, Ordering::Release);
        let _ = watcher.join();
        caller.check()?;
        result
    })();
    let response = match result {
        Ok(value) => Response {
            ipc_version: crate::IPC_VERSION,
            server_version: crate::VERSION.into(),
            correlation_id: id,
            result: Some(value),
            error: None,
        },
        Err(error) => Response {
            ipc_version: crate::IPC_VERSION,
            server_version: crate::VERSION.into(),
            correlation_id: id,
            result: None,
            error: Some(error),
        },
    };
    let value = serde_json::to_value(response)?;
    if serde_json::to_vec(&value)?.len() + 1 > crate::MAX_MESSAGE_BYTES {
        return framing::write_frame(
            &mut stream,
            &json!({"ipc_version":crate::IPC_VERSION,"server_version":crate::VERSION,"correlation_id":value["correlation_id"],"result":null,"error":{"code":"LIMIT_EXCEEDED","message":"Response exceeds the IPC byte limit"}}),
            crate::MAX_MESSAGE_BYTES,
        );
    }
    framing::write_frame(&mut stream, &value, crate::MAX_MESSAGE_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precancelled_request_never_connects_or_reads_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let profile = Profile::new(temp.path().join("profile")).unwrap();
        let client = Client::new(profile, "owner", true);
        let result = client.invoke_cancelled("status", json!({}), Arc::new(AtomicBool::new(true)));
        assert_eq!(result.unwrap_err().code, "CANCELLED");
        assert!(!client.profile.database().exists());
    }
}

#[cfg(all(test, unix))]
mod serve_tests {
    use super::*;
    use crate::answer::llm::UnavailableModel;
    use crate::storage::Store;
    use std::io::Write;

    fn exchange(stream: &mut Stream, value: &Value) -> Value {
        framing::write_frame(&mut *stream, value, crate::MAX_MESSAGE_BYTES).unwrap();
        let bytes = framing::read_frame(&mut BufReader::new(stream), crate::MAX_MESSAGE_BYTES)
            .unwrap()
            .expect("the Core answers every well-formed request exactly once");
        serde_json::from_slice(&bytes).unwrap()
    }
    fn envelope(token: &str, operation: &str, input: Value) -> Value {
        json!({"ipc_version":crate::IPC_VERSION,"client_version":crate::VERSION,"correlation_id":"request:test","deadline_ms":now_ms()+30_000,"token":token,"operation":operation,"input":input})
    }
    fn connect(profile: &Profile) -> Stream {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match Stream::connect(profile) {
                Ok(stream) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    break stream;
                }
                Err(error) if error.code == "CORE_UNAVAILABLE" && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => panic!("Core socket did not become ready: {error}"),
            }
        }
    }

    #[test]
    fn a_client_never_sees_the_socket_before_it_is_private() {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        let temporary = tempfile::tempdir().unwrap();
        let profile = Profile::new(temporary.path().join("profile")).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let watcher = {
            let (socket, done) = (profile.socket(), done.clone());
            // What a client checks before it connects, as often as it can.
            std::thread::spawn(move || {
                let mut unsafe_seen = 0;
                while !done.load(Ordering::Acquire) {
                    if let Ok(meta) = std::fs::symlink_metadata(&socket) {
                        if meta.file_type().is_socket() && meta.permissions().mode() & 0o077 != 0 {
                            unsafe_seen += 1;
                        }
                    }
                }
                unsafe_seen
            })
        };
        for _ in 0..300 {
            drop(Listener::bind(&profile).unwrap());
        }
        done.store(true, Ordering::Release);
        assert_eq!(
            watcher.join().unwrap(),
            0,
            "the socket was visible before it was private"
        );
    }

    #[test]
    fn core_serves_authenticated_clients_and_rejects_bad_tokens_and_frames() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = Profile::new(temporary.path().join("profile")).unwrap();
        let mut store = Store::open(profile.clone()).unwrap();
        let owner = "o".repeat(64);
        let mcp = "m".repeat(64);
        store.bootstrap_credentials(&owner, &mcp).unwrap();
        let app = App::new(store, Arc::new(UnavailableModel));
        let shutdown = Arc::new(AtomicBool::new(false));
        let server = {
            let (profile, shutdown) = (profile.clone(), shutdown.clone());
            std::thread::spawn(move || serve(app, profile, 0, shutdown))
        };

        // Authenticated owner request through the real framing/authentication/dispatch path.
        let response = exchange(
            &mut connect(&profile),
            &envelope(&owner, "status", json!({})),
        );
        assert_eq!(response["correlation_id"], "request:test");
        assert_eq!(response["result"]["product"], crate::APP_NAME);
        assert_eq!(response["result"]["health"], "GRAPH_ONLY");
        assert!(response["error"].is_null());

        // Unknown token: no caller, no dispatch, redacted error.
        let response = exchange(
            &mut connect(&profile),
            &envelope(&"x".repeat(64), "status", json!({})),
        );
        assert_eq!(response["error"]["code"], "AUTH_REQUIRED");

        // The MCP principal is authenticated but lacks the admin permission for backup.
        let response = exchange(
            &mut connect(&profile),
            &envelope(&mcp, "backup", json!({"name":"x"})),
        );
        assert_eq!(response["error"]["code"], "PERMISSION_DENIED");
        assert!(!profile.directory().join("backup-x.db").exists());

        // Request JSON cannot carry caller authority or unknown envelope fields.
        let mut forged = envelope(&mcp, "status", json!({}));
        forged["permissions"] = json!(["admin"]);
        let response = exchange(&mut connect(&profile), &forged);
        assert_eq!(response["error"]["code"], "INVALID_INPUT");

        // Malformed JSON still gets a bounded error frame, not a hung connection.
        let mut stream = connect(&profile);
        stream.write_all(b"{not json\n").unwrap();
        let bytes = framing::read_frame(&mut BufReader::new(&mut stream), crate::MAX_MESSAGE_BYTES)
            .unwrap()
            .unwrap();
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response["error"]["code"], "INVALID_INPUT");

        // An oversized frame is cut off at the byte limit; the Core closes that connection
        // without dispatching and keeps serving other clients.
        let mut stream = connect(&profile);
        let oversized = vec![b'a'; crate::MAX_MESSAGE_BYTES + 1];
        let _ = stream.write_all(&oversized);
        let _ = stream.write_all(b"\n");
        let closed =
            framing::read_frame(&mut BufReader::new(&mut stream), crate::MAX_MESSAGE_BYTES);
        assert!(matches!(closed, Ok(None) | Err(_)));
        let response = exchange(
            &mut connect(&profile),
            &envelope(&owner, "status", json!({})),
        );
        assert_eq!(response["result"]["product"], crate::APP_NAME);

        // Concurrent clients share the single Core without a second writer or deadlock.
        let workers: Vec<_> = (0..6)
            .map(|_| {
                let (profile, owner) = (profile.clone(), owner.clone());
                std::thread::spawn(move || {
                    exchange(
                        &mut connect(&profile),
                        &envelope(&owner, "list_relation_types", json!({})),
                    )
                })
            })
            .collect();
        for worker in workers {
            let response = worker.join().unwrap();
            assert!(response["result"]
                .as_array()
                .is_some_and(|types| !types.is_empty()));
        }

        shutdown.store(true, Ordering::Release);
        server.join().unwrap().unwrap();
        assert!(!profile.socket().exists());
        assert_eq!(Store::open(profile).unwrap().epochs().unwrap().policy, 1);
    }

    #[test]
    fn a_client_reads_its_credential_once_and_follows_a_rotation() {
        use crate::runtime::auth::{memory, rotate_credential, OsSecrets};
        let temporary = tempfile::tempdir().unwrap();
        let profile = Profile::new(temporary.path().join("profile")).unwrap();
        let mut store = Store::open(profile.clone()).unwrap();
        let owner = OsSecrets::create(&profile, "owner").unwrap();
        let mcp = OsSecrets::create(&profile, "mcp").unwrap();
        store.bootstrap_credentials(&owner, &mcp).unwrap();
        let app = App::new(store, Arc::new(UnavailableModel));
        let shutdown = Arc::new(AtomicBool::new(false));
        let server = {
            let (app, profile, shutdown) = (app.clone(), profile.clone(), shutdown.clone());
            std::thread::spawn(move || serve(app, profile, 0, shutdown))
        };
        drop(connect(&profile));
        let reads = || memory::reads(&profile, "owner");
        let client = Client::new(profile.clone(), "owner", false);

        // One read serves every request of this client and of its clones.
        for asker in [client.clone(), client.clone(), client.clone()] {
            asker.invoke("status", json!({})).unwrap();
        }
        assert_eq!(reads(), 1);

        // A rotation elsewhere: the remembered credential is refused, read again, and sent once.
        rotate_credential(&mut app.store().unwrap(), &profile, "owner").unwrap();
        client.invoke("status", json!({})).unwrap();
        assert_eq!(reads(), 2);
        client.invoke("status", json!({})).unwrap();
        assert_eq!(reads(), 2);

        // A stored credential the profile does not accept is refused once, with the way out.
        memory::put(&profile, "owner", &format!("cap:{}", "0".repeat(64)));
        client.forget_credential();
        let refused = client.invoke("status", json!({})).unwrap_err();
        assert_eq!(refused.code, "AUTH_REQUIRED");
        assert!(
            refused.message.contains("huda credentials restore"),
            "{}",
            refused.message
        );
        assert_eq!(reads(), 3, "a credential just read is not read again");

        // A cancelled request reads nothing.
        client.forget_credential();
        let cancelled =
            client.invoke_cancelled("status", json!({}), Arc::new(AtomicBool::new(true)));
        assert_eq!(cancelled.unwrap_err().code, "CANCELLED");
        assert_eq!(reads(), 3);

        shutdown.store(true, Ordering::Release);
        server.join().unwrap().unwrap();
    }
}
