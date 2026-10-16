//! Modern MCP stdio only. JSON framing adapts to authenticated IPC; it owns no database.
use crate::api::contracts;
use crate::error::{Error, Result};
use crate::runtime::{framing, ipc::Client};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub fn request_meta() -> Value {
    json!({"io.modelcontextprotocol/protocolVersion":crate::MCP_VERSION,"io.modelcontextprotocol/clientInfo":{"name":"Huda","version":crate::VERSION},"io.modelcontextprotocol/clientCapabilities":{}})
}
fn server_meta() -> Value {
    json!({"io.modelcontextprotocol/serverInfo":{"name":"Huda","version":crate::VERSION}})
}
fn rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code":code,"message":message});
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({"jsonrpc":"2.0","id":id,"error":error})
}
fn valid_id(id: &Value) -> bool {
    id.is_string() || id.is_i64() || id.is_u64()
}
/// Protocol validation is deterministic and independent of domain implementation.
fn validate(request: &Value) -> std::result::Result<(), Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || !request.get("method").is_some_and(Value::is_string)
        || !valid_id(&id)
    {
        return Err(rpc_error(
            Value::Null,
            -32600,
            "Invalid JSON-RPC request",
            None,
        ));
    }
    if request["method"] == "initialize" {
        return Err(rpc_error(
            id,
            -32601,
            "Huda supports MCP 2026-07-28 server/discover; initialize is not supported",
            None,
        ));
    }
    let Some(meta) = request.pointer("/params/_meta").and_then(Value::as_object) else {
        return Err(rpc_error(
            id,
            -32602,
            "Required per-request MCP metadata is missing",
            None,
        ));
    };
    let Some(version) = meta
        .get("io.modelcontextprotocol/protocolVersion")
        .and_then(Value::as_str)
    else {
        return Err(rpc_error(
            id,
            -32602,
            "Protocol version metadata is required",
            None,
        ));
    };
    if version != crate::MCP_VERSION {
        return Err(rpc_error(
            id,
            -32022,
            "Unsupported protocol version",
            Some(json!({"supported":[crate::MCP_VERSION],"requested":version})),
        ));
    }
    if !meta
        .get("io.modelcontextprotocol/clientCapabilities")
        .is_some_and(Value::is_object)
    {
        return Err(rpc_error(
            id,
            -32602,
            "Client capability metadata must be an object",
            None,
        ));
    }
    Ok(())
}
fn handle(request: Value, client: &Client, cancelled: Arc<AtomicBool>) -> Value {
    if let Err(error) = validate(&request) {
        return error;
    }
    let id = request["id"].clone();
    let mut result = match request["method"].as_str().unwrap_or("") {
        // DiscoverResult and ListToolsResult require cacheScope and ttlMs in 2026-07-28.
        // The catalog is static and contains no caller data, but it is served only to
        // this authenticated local process, so it is declared private and immediately stale.
        "server/discover" => {
            json!({"supportedVersions":[crate::MCP_VERSION],"capabilities":{"tools":{}},"cacheScope":"private","ttlMs":0,"instructions":"Huda is relation-first. Evidence is optional, origin and authorization are not. External access requires prior independent consent. Persistent inference is disabled."})
        }
        "tools/list" => {
            if request["params"]
                .get("cursor")
                .is_some_and(|v| !v.is_null())
            {
                return rpc_error(id, -32602, "This bounded catalog has no cursor", None);
            }
            json!({"tools":contracts::tools(),"cacheScope":"private","ttlMs":0})
        }
        "tools/call" => {
            let Some(name) = request.pointer("/params/name").and_then(Value::as_str) else {
                return rpc_error(id, -32602, "Tool name is required", None);
            };
            let Some(operation) = contracts::tool_operation(name) else {
                return rpc_error(id, -32602, "Tool is not in the Huda catalog", None);
            };
            let arguments = request
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if !arguments.is_object() {
                return rpc_error(id, -32602, "Tool arguments must be an object", None);
            }
            match client.invoke_cancelled(operation, arguments, cancelled) {
                Ok(value) => {
                    json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":false})
                }
                Err(error) => {
                    let value = json!({"code":error.code,"message":error.message});
                    json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":true})
                }
            }
        }
        _ => {
            return rpc_error(
                id,
                -32601,
                "Method is not supported by Huda MCP 2026-07-28",
                None,
            )
        }
    };
    result["resultType"] = "complete".into();
    result["_meta"] = server_meta();
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

pub fn stdio(client: Client) -> Result<()> {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let output = Arc::new(Mutex::new(std::io::stdout()));
    let active: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>> =
        Arc::new(Mutex::new(BTreeMap::new()));
    let mut workers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    let outcome = (|| -> Result<()> {
        while let Some(frame) = framing::read_frame(&mut input, crate::MAX_MESSAGE_BYTES)? {
            let request: Value = match serde_json::from_slice(&frame) {
                Ok(value) => value,
                Err(_) => {
                    write(
                        &output,
                        &rpc_error(Value::Null, -32700, "Parse error", None),
                    )?;
                    continue;
                }
            };
            if request.get("id").is_none() {
                if request["jsonrpc"] == "2.0" && request["method"] == "notifications/cancelled" {
                    if let Some(id) = request
                        .pointer("/params/requestId")
                        .filter(|id| valid_id(id))
                    {
                        if let Some(flag) = active
                            .lock()
                            .map_err(|_| {
                                Error::new("INTERNAL_ERROR", "MCP cancellation state unavailable")
                            })?
                            .get(&id.to_string())
                        {
                            flag.store(true, Ordering::Release);
                        }
                    }
                }
                // Never execute tool calls delivered as notifications. Notifications get no response.
                continue;
            }
            if let Err(error) = validate(&request) {
                write(&output, &error)?;
                continue;
            }
            let key = request["id"].to_string();
            let flag = Arc::new(AtomicBool::new(false));
            {
                let mut active = active
                    .lock()
                    .map_err(|_| Error::new("INTERNAL_ERROR", "MCP request state unavailable"))?;
                if active.contains_key(&key) || active.len() >= 8 {
                    write(
                        &output,
                        &rpc_error(
                            request["id"].clone(),
                            -32000,
                            "Duplicate request ID or MCP backpressure",
                            None,
                        ),
                    )?;
                    continue;
                }
                active.insert(key.clone(), flag.clone());
            }
            // Reap completed handles so a long-lived MCP host does not grow an unbounded list.
            let mut index = 0;
            while index < workers.len() {
                if workers[index].is_finished() {
                    let worker = workers.swap_remove(index);
                    let _ = worker.join();
                } else {
                    index += 1;
                }
            }
            let client = client.clone();
            let active = active.clone();
            let output = output.clone();
            workers.push(std::thread::spawn(move || {
                let id = request["id"].clone();
                // A panicking handler still answers its request and frees its slot below.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    handle(request, &client, flag.clone())
                }))
                .unwrap_or_else(|_| rpc_error(id, -32603, "Internal error", None));
                if !flag.load(Ordering::Acquire) {
                    let _ = write(&output, &result);
                }
                if let Ok(mut active) = active.lock() {
                    active.remove(&key);
                }
            }));
        }
        Ok(())
    })();
    if let Ok(active) = active.lock() {
        for flag in active.values() {
            flag.store(true, Ordering::Release);
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
    outcome
}
fn write(output: &Arc<Mutex<std::io::Stdout>>, value: &Value) -> Result<()> {
    let mut writer = output
        .lock()
        .map_err(|_| Error::new("INTERNAL_ERROR", "MCP output stream unavailable"))?;
    if serde_json::to_vec(value)?.len() + 1 > crate::MAX_MESSAGE_BYTES {
        return framing::write_frame(
            &mut *writer,
            &rpc_error(
                value["id"].clone(),
                -32000,
                "Response exceeds the Huda byte bound",
                None,
            ),
            crate::MAX_MESSAGE_BYTES,
        );
    }
    framing::write_frame(&mut *writer, value, crate::MAX_MESSAGE_BYTES)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stateless_metadata_is_required() {
        assert!(
            validate(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})).is_err()
        );
        assert!(validate(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":request_meta()}})
        )
        .is_ok());
    }
    #[test]
    fn discover_and_catalog_results_carry_schema_required_fields() {
        // server/discover and tools/list never reach IPC, so an unreachable profile is fine.
        let directory = tempfile::tempdir().unwrap();
        let client = Client {
            profile: crate::runtime::Profile::new(directory.path().join("profile")).unwrap(),
            actor: "mcp".into(),
            start_on_demand: false,
        };
        for method in ["server/discover", "tools/list"] {
            let request =
                json!({"jsonrpc":"2.0","id":7,"method":method,"params":{"_meta":request_meta()}});
            let response = handle(request, &client, Arc::new(AtomicBool::new(false)));
            let result = &response["result"];
            assert_eq!(response["id"], 7, "{method}");
            assert_eq!(result["resultType"], "complete", "{method}");
            assert_eq!(result["cacheScope"], "private", "{method}");
            assert_eq!(result["ttlMs"], 0, "{method}");
            assert_eq!(
                result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
                "Huda"
            );
        }
        let discover = handle(
            json!({"jsonrpc":"2.0","id":"d","method":"server/discover","params":{"_meta":request_meta()}}),
            &client,
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(
            discover["result"]["supportedVersions"],
            json!([crate::MCP_VERSION])
        );
        assert!(discover["result"]["capabilities"]["tools"].is_object());
        let tools = handle(
            json!({"jsonrpc":"2.0","id":"t","method":"tools/list","params":{"_meta":request_meta()}}),
            &client,
            Arc::new(AtomicBool::new(false)),
        );
        let tools = tools["result"]["tools"].as_array().unwrap();
        assert!(!tools.is_empty());
        for tool in tools {
            assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
        }
        assert!(!client.profile.database().exists());
    }
    #[test]
    fn no_silent_protocol_fallback() {
        let mut metadata = request_meta();
        metadata["io.modelcontextprotocol/protocolVersion"] = "2025-11-25".into();
        let error = validate(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":metadata}}),
        )
        .unwrap_err();
        assert_eq!(error["error"]["code"], -32022);
    }
}
