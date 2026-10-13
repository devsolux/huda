//! Opt-in, authenticated loopback REST. A strict HTTP/1.1 JSON subset parsed by httparse.
//! One bounded request per connection; no browser origin, anonymous route, chunked upload,
//! proxy trust, public bind, WebSocket, SSE, or implicit MCP-over-HTTP compatibility claim.
use super::{contracts, App};
use crate::error::{require, Error, Result};
use crate::relation::{identifier, now_ms, random_id};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

pub struct RestServer {
    listener: TcpListener,
    port: u16,
}
impl RestServer {
    pub fn bind(port: u16) -> Result<Self> {
        require(
            port != 0,
            "INVALID_INPUT",
            "Choose an explicit nonzero loopback port",
        )?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, port })
    }
    fn accept(&self) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
        let (stream, peer) = self.listener.accept()?;
        // BSD/macOS accepted sockets inherit O_NONBLOCK from the polling listener. Request
        // deadlines and the disconnect watcher rely on blocking I/O bounded by timeouts.
        stream.set_nonblocking(false)?;
        Ok((stream, peer))
    }
    pub fn serve(self, app: App, shutdown: Arc<AtomicBool>) -> Result<()> {
        let active = Arc::new(AtomicUsize::new(0));
        while !shutdown.load(Ordering::Acquire) {
            match self.accept() {
                Ok((stream, peer)) => {
                    if !peer.ip().is_loopback() {
                        drop(stream);
                        continue;
                    }
                    if active
                        .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                            (n < 16).then_some(n + 1)
                        })
                        .is_err()
                    {
                        drop(stream);
                        continue;
                    }
                    let app = app.clone();
                    let active = active.clone();
                    let port = self.port;
                    std::thread::spawn(move || {
                        struct Slot(Arc<AtomicUsize>);
                        impl Drop for Slot {
                            fn drop(&mut self) {
                                self.0.fetch_sub(1, Ordering::AcqRel);
                            }
                        }
                        let _slot = Slot(active);
                        let _ = handle(&app, stream, port);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}
struct Head {
    verb: String,
    target: String,
    headers: BTreeMap<String, String>,
    body_at: usize,
    length: usize,
}
fn parse_head(bytes: &[u8]) -> Result<Option<Head>> {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut request = httparse::Request::new(&mut headers);
    let size = match request
        .parse(bytes)
        .map_err(|_| Error::new("BAD_REQUEST", "Malformed HTTP request"))?
    {
        httparse::Status::Partial => return Ok(None),
        httparse::Status::Complete(size) => size,
    };
    require(
        request.version == Some(1),
        "BAD_REQUEST",
        "Only HTTP/1.1 is supported",
    )?;
    let mut parsed = BTreeMap::new();
    for header in request.headers.iter() {
        let key = header.name.to_ascii_lowercase();
        let value = std::str::from_utf8(header.value)
            .map_err(|_| Error::new("BAD_REQUEST", "Header encoding is invalid"))?
            .trim();
        require(
            value.bytes().all(|byte| (0x20..0x7f).contains(&byte)),
            "BAD_REQUEST",
            "Header contains invalid control characters",
        )?;
        require(
            parsed.insert(key, value.to_owned()).is_none(),
            "BAD_REQUEST",
            "Duplicate headers are not accepted",
        )?;
    }
    require(
        !parsed.contains_key("transfer-encoding")
            && !parsed.contains_key("expect")
            && !parsed.contains_key("upgrade"),
        "BAD_REQUEST",
        "Chunked, interim and upgraded requests are not supported",
    )?;
    let length = match parsed.get("content-length") {
        Some(value) => {
            require(
                !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
                "BAD_REQUEST",
                "Invalid content length",
            )?;
            value
                .parse::<usize>()
                .map_err(|_| Error::new("LIMIT_EXCEEDED", "Content length exceeds bounds"))?
        }
        None => 0,
    };
    require(
        size <= 8192 && length <= crate::MAX_MESSAGE_BYTES,
        "LIMIT_EXCEEDED",
        "HTTP header/body exceeds its byte limit",
    )?;
    Ok(Some(Head {
        verb: request.method.unwrap_or("").to_owned(),
        target: request.path.unwrap_or("").to_owned(),
        headers: parsed,
        body_at: size,
        length,
    }))
}
fn read_chunk(
    stream: &mut TcpStream,
    buffer: &mut Vec<u8>,
    deadline: Instant,
    max: usize,
) -> Result<()> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| Error::new("DEADLINE_EXCEEDED", "Request read deadline exceeded"))?;
    require(
        !remaining.is_zero(),
        "DEADLINE_EXCEEDED",
        "Request read deadline exceeded",
    )?;
    stream.set_read_timeout(Some(remaining))?;
    let mut chunk = [0u8; 1024];
    let limit = chunk.len().min(max);
    require(limit > 0, "LIMIT_EXCEEDED", "HTTP read exceeds bounds")?;
    let n = stream
        .read(&mut chunk[..limit])
        .map_err(|_| Error::new("BAD_REQUEST", "Incomplete or timed-out HTTP body"))?;
    require(
        n != 0,
        "BAD_REQUEST",
        "Connection closed before request completion",
    )?;
    buffer.extend_from_slice(&chunk[..n]);
    Ok(())
}
fn handle(app: &App, mut stream: TcpStream, port: u16) -> Result<()> {
    let correlation = random_id("http")?;
    let result = (|| -> Result<(u16, Value)> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut bytes = Vec::new();
        let head = loop {
            if let Some(head) = parse_head(&bytes)? {
                break head;
            }
            require(
                bytes.len() < 8192,
                "LIMIT_EXCEEDED",
                "HTTP header is too large",
            )?;
            read_chunk(&mut stream, &mut bytes, deadline, 8192)?;
        };
        require(
            head.headers.get("host").map(String::as_str)
                == Some(format!("127.0.0.1:{port}").as_str()),
            "PERMISSION_DENIED",
            "Host is not the explicitly bound loopback authority",
        )?;
        require(
            !head.headers.contains_key("origin") && !head.headers.contains_key("cookie"),
            "PERMISSION_DENIED",
            "Browser-origin and cookie-authenticated requests are not accepted",
        )?;
        let token = head
            .headers
            .get("authorization")
            .and_then(|header| header.strip_prefix("Bearer "))
            .ok_or_else(|| {
                Error::new("AUTH_REQUIRED", "Scoped bearer authentication is required")
            })?;
        let caller = app.authenticate(token, now_ms() + 120_000)?;
        require(
            matches!(head.verb.as_str(), "GET" | "POST" | "DELETE"),
            "METHOD_NOT_ALLOWED",
            "Unsupported HTTP method",
        )?;
        if head.verb == "POST" {
            require(
                head.length > 0
                    && head
                        .headers
                        .get("content-type")
                        .is_some_and(|s| s.eq_ignore_ascii_case("application/json")),
                "BAD_REQUEST",
                "POST requires application/json and a positive Content-Length",
            )?;
        } else {
            require(
                head.length == 0,
                "BAD_REQUEST",
                "GET/DELETE bodies are not supported; use documented query parameters",
            )?;
        }
        let total = head.body_at + head.length;
        require(
            bytes.len() <= total,
            "BAD_REQUEST",
            "Pipelined requests are not supported",
        )?;
        while bytes.len() < total {
            let remaining = total - bytes.len();
            read_chunk(&mut stream, &mut bytes, deadline, remaining)?;
        }
        let (operation, path_args) = route(&head.verb, &head.target)?;
        let mut payload = if head.verb == "POST" {
            serde_json::from_slice::<Value>(&bytes[head.body_at..])?
        } else {
            Value::Object(Map::new())
        };
        let object = payload
            .as_object_mut()
            .ok_or_else(|| Error::invalid("Request body must be an object"))?;
        for (key, value) in path_args {
            require(
                object.insert(key, value).is_none(),
                "BAD_REQUEST",
                "Path/query/body fields cannot override each other",
            )?;
        }
        // A disconnect cancels in-flight work before the publication checkpoint; already
        // committed transactions and separately requested async jobs are not undone.
        let monitor = stream.try_clone()?;
        monitor.set_read_timeout(Some(Duration::from_millis(25)))?;
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let cancelled = caller.cancelled.clone();
        let watcher = std::thread::spawn(move || {
            let mut byte = [0];
            while !finished.load(Ordering::Acquire) {
                match monitor.peek(&mut byte) {
                    Ok(_) => {
                        cancelled.store(true, Ordering::Release);
                        break;
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => {
                        cancelled.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        });
        let result = app.dispatch(&caller, operation, payload);
        done.store(true, Ordering::Release);
        let _ = watcher.join();
        let result = result?;
        caller.check()?;
        let status = if matches!(
            operation,
            "ingest_source" | "expand_knowledge" | "capture_bridge"
        ) {
            202
        } else {
            200
        };
        Ok((status, result))
    })();
    let (status, value) = match result {
        Ok(value) => value,
        Err(error) => (
            error.status(),
            json!({"code":error.code,"message":error.message,"correlation_id":correlation}),
        ),
    };
    write_response(&mut stream, status, &value, &correlation)
}
fn route(verb: &str, target: &str) -> Result<(&'static str, Map<String, Value>)> {
    require(
        target.starts_with('/')
            && !target.starts_with("//")
            && !target.contains('#')
            && target.len() <= 4096,
        "BAD_REQUEST",
        "Invalid request target",
    )?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut fields = Map::new();
    let query_url = reqwest::Url::parse(&format!("http://127.0.0.1/?{query}"))
        .map_err(|_| Error::new("BAD_REQUEST", "Invalid query encoding"))?;
    for (key, value) in query_url.query_pairs() {
        require(
            !fields.contains_key(key.as_ref()),
            "BAD_REQUEST",
            "Duplicate query parameter",
        )?;
        // Numeric query fields map to the same typed DTOs as CLI/IPC/MCP input.
        let typed = if key.starts_with("expected_") || key == "tx_as_of" {
            json!(value
                .parse::<u64>()
                .map_err(|_| Error::invalid("Expected an unsigned integer query parameter"))?)
        } else if value == "true" || value == "false" {
            json!(value == "true")
        } else {
            json!(value)
        };
        fields.insert(key.into_owned(), typed);
    }
    if verb == "GET" && path == "/openapi.json" {
        require(
            fields.is_empty(),
            "INVALID_INPUT",
            "OpenAPI takes no query parameters",
        )?;
        return Ok(("openapi", fields));
    }
    for operation in contracts::OPERATIONS
        .iter()
        .filter(|operation| operation.verb.eq_ignore_ascii_case(verb))
    {
        let expected: Vec<_> = operation.path.split('/').collect();
        let actual: Vec<_> = path.split('/').collect();
        if expected.len() != actual.len() {
            continue;
        }
        let mut values = Map::new();
        let mut matched = true;
        for (pattern, segment) in expected.iter().zip(&actual) {
            if pattern.starts_with('{') && pattern.ends_with('}') {
                let key = &pattern[1..pattern.len() - 1];
                let encoded = reqwest::Url::parse(&format!("http://127.0.0.1/?value={segment}"))
                    .map_err(|_| Error::invalid("Invalid path identifier"))?;
                let value = encoded
                    .query_pairs()
                    .next()
                    .map(|(_, value)| value.into_owned())
                    .ok_or_else(Error::not_found)?;
                identifier(&value)?;
                values.insert(key.into(), json!(value));
            } else if pattern != segment {
                matched = false;
                break;
            }
        }
        if matched {
            for (key, value) in values {
                require(
                    fields.insert(key, value).is_none(),
                    "BAD_REQUEST",
                    "Query cannot override a path identifier",
                )?;
            }
            return Ok((operation.name, fields));
        }
    }
    Err(Error::not_found())
}
fn write_response(
    stream: &mut TcpStream,
    mut status: u16,
    value: &Value,
    correlation: &str,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() > crate::MAX_MESSAGE_BYTES {
        status = 413;
        bytes = serde_json::to_vec(
            &json!({"code":"LIMIT_EXCEEDED","message":"Response exceeds HTTP byte limit","correlation_id":correlation}),
        )?;
    }
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Content Too Large",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    write!(stream,"HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Correlation-Id: {correlation}\r\n\r\n",bytes.len())?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer::llm::UnavailableModel;
    use crate::runtime::Profile;
    use crate::storage::Store;
    use std::net::Ipv4Addr;

    #[test]
    fn loopback_http_requires_exact_host_and_bearer_and_shares_handlers() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = Profile::new(temporary.path().join("profile")).unwrap();
        let mut store = Store::open(profile).unwrap();
        let owner = "o".repeat(64);
        store
            .bootstrap_credentials(&owner, &"m".repeat(64))
            .unwrap();
        let app = App::new(store, Arc::new(UnavailableModel));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let host = format!("127.0.0.1:{port}");
        let exchange = |request: String| -> (u16, Value) {
            let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            let (server, _) = listener.accept().unwrap();
            let app = app.clone();
            let worker = std::thread::spawn(move || handle(&app, server, port));
            client.write_all(request.as_bytes()).unwrap();
            let mut raw = Vec::new();
            client.read_to_end(&mut raw).unwrap();
            worker.join().unwrap().unwrap();
            let text = String::from_utf8(raw).unwrap();
            let (head, body) = text.split_once("\r\n\r\n").unwrap();
            assert!(head.contains("\r\nX-Correlation-Id: http:"));
            let status: u16 = head.split(' ').nth(1).unwrap().parse().unwrap();
            (status, serde_json::from_str(body).unwrap())
        };
        let get = |path: &str, host: &str, token: Option<&str>| {
            let auth = token
                .map(|t| format!("Authorization: Bearer {t}\r\n"))
                .unwrap_or_default();
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{auth}\r\n")
        };

        let (status, body) = exchange(get("/api/v1/status", &host, Some(&owner)));
        assert_eq!(status, 200);
        assert_eq!(body["product"], crate::APP_NAME);
        let epoch = body["epochs"]["policy"].as_u64().unwrap();

        let (status, body) = exchange(get("/api/v1/status", &host, None));
        assert_eq!(
            (status, body["code"].as_str()),
            (401, Some("AUTH_REQUIRED"))
        );
        assert!(body["correlation_id"]
            .as_str()
            .is_some_and(|c| c.starts_with("http:")));
        let (status, _) = exchange(get("/api/v1/status", "localhost:1", Some(&owner)));
        assert_eq!(status, 403);
        let (status, _) = exchange(format!(
            "GET /api/v1/status HTTP/1.1\r\nHost: {host}\r\nOrigin: http://127.0.0.1\r\nAuthorization: Bearer {owner}\r\n\r\n"
        ));
        assert_eq!(status, 403);
        let (status, _) = exchange(get("/api/v1/nope", &host, Some(&owner)));
        assert_eq!(status, 404);
        let (status, _) = exchange(format!(
            "PUT /api/v1/status HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {owner}\r\n\r\n"
        ));
        assert_eq!(status, 405);

        // The same handler and DTO as IPC/MCP: a REST write produces a canonical entity.
        let payload = json!({"entity":{"namespace":"demo","kind":"concept","canonical_key":"a","name":"A","aliases":[]},"scope":"local","expected_policy_epoch":epoch}).to_string();
        let (status, body) = exchange(format!(
            "POST /api/v1/entities HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {owner}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
            payload.len()
        ));
        assert_eq!(status, 200, "{body}");
        assert!(body["entity_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("ent:")));
        let (status, body) = exchange(get("/api/v1/relation-types", &host, Some(&owner)));
        assert_eq!(status, 200);
        assert!(body.as_array().is_some_and(|types| !types.is_empty()));
    }
    #[test]
    fn accepted_connection_blocks_until_late_request_bytes_arrive() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = RestServer { listener, port };
        let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let (mut stream, _) = loop {
            match server.accept() {
                Ok(accepted) => break accepted,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("Loopback accept failed: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            client.write_all(b"late").unwrap();
            client
        });
        let mut input = [0u8; 4];
        stream.read_exact(&mut input).unwrap();
        assert_eq!(&input, b"late");
        drop(writer.join().unwrap());
    }
    #[test]
    fn rejects_ambiguous_framing() {
        for input in [
            "GET / HTTP/1.1\r\nHost: x\r\nHost: y\r\n\r\n",
            "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            "POST / HTTP/1.1\r\nContent-Length: -1\r\n\r\n",
        ] {
            assert!(parse_head(input.as_bytes()).is_err());
        }
    }
    #[test]
    fn routes_to_shared_use_case() {
        assert_eq!(
            route("POST", "/api/v1/relations").unwrap().0,
            "assert_relation"
        );
        assert_eq!(
            route("POST", "/api/v1/knowledge/expand").unwrap().0,
            "expand_knowledge"
        );
        assert!(route("DELETE", "/api/v1/assertions/a?assertion_id=b").is_err());
    }
}
