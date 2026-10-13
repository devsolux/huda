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
    Arc,
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
#[derive(Clone)]
pub struct Client {
    pub profile: Profile,
    pub actor: String,
    pub start_on_demand: bool,
}
impl Client {
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
        let token = OsSecrets::get(&self.profile, &self.actor)?;
        check_cancelled(&cancelled)?;
        let mut stream = self.connect(&cancelled)?;
        stream.verify_peer()?;
        check_cancelled(&cancelled)?;
        let correlation_id = random_id("request")?;
        let envelope = Envelope {
            ipc_version: crate::IPC_VERSION,
            client_version: crate::VERSION.into(),
            correlation_id: correlation_id.clone(),
            deadline_ms: now_ms() + 120_000,
            token,
            operation: operation.into(),
            input,
        };
        stream.set_read_timeout(Some(Duration::from_secs(120)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        let monitor = stream.try_clone()?;
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let signal = cancelled.clone();
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
            framing::write_frame(
                &mut stream,
                &serde_json::to_value(envelope)?,
                crate::MAX_MESSAGE_BYTES,
            )?;
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
        // Reap our child; never kill a shared Core just because this client closes.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            check_cancelled(cancelled)?;
            match Stream::connect(&self.profile) {
                Ok(stream) => return Ok(stream),
                Err(error) if error.code == "CORE_UNAVAILABLE" => {}
                Err(error) => return Err(error),
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err(Error::new(
            "CORE_UNAVAILABLE",
            "Core did not become ready; run huda core in the foreground for its redacted error",
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
    let mut listener = Listener::bind(&profile)?;
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
        let client = Client {
            profile,
            actor: "owner".into(),
            start_on_demand: true,
        };
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
}
