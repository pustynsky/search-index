use std::io::{self, BufRead, Write};
use std::sync::{Arc, RwLock};

use serde_json::Value;
use tracing::{debug, error, info, warn};

use crate::mcp::handlers::{self, HandlerContext};
use crate::mcp::protocol::*;
use crate::ContentIndex;
use crate::definitions::DefinitionIndex;

/// Run the MCP server event loop over stdio
pub fn run_server(
    index: Arc<RwLock<ContentIndex>>,
    def_index: Option<Arc<RwLock<DefinitionIndex>>>,
    server_dir: String,
    server_ext: String,
) {
    let ctx = HandlerContext {
        index,
        def_index,
        server_dir,
        server_ext,
    };

    let stdin = io::stdin();
    let reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();

    info!("MCP server ready, waiting for JSON-RPC requests on stdin");

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                error!(error = %e, "Error reading stdin");
                break;
            }
        };

        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        debug!(request = %line, "Incoming JSON-RPC");

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "Failed to parse JSON-RPC request");
                let err = JsonRpcErrorResponse::new(
                    Value::Null,
                    -32700,
                    format!("Parse error: {}", e),
                );
                let resp = serde_json::to_string(&err).unwrap();
                debug!(response = %resp, "Error response");
                let _ = writeln!(writer, "{}", resp);
                let _ = writer.flush();
                continue;
            }
        };

        // Notifications have no id — don't send a response
        if request.id.is_none() {
            debug!(method = %request.method, "Received notification");
            continue;
        }

        let id = request.id.unwrap();
        let response = handle_request(&ctx, &request.method, &request.params, id.clone());

        let resp_str = serde_json::to_string(&response).unwrap();
        debug!(response = %resp_str, "Outgoing JSON-RPC");
        let _ = writeln!(writer, "{}", resp_str);
        let _ = writer.flush();
    }

    info!("stdin closed, shutting down");
}

fn handle_request(
    ctx: &HandlerContext,
    method: &str,
    params: &Option<Value>,
    id: Value,
) -> Value {
    match method {
        "initialize" => {
            let result = InitializeResult::new();
            serde_json::to_value(JsonRpcResponse::new(
                id,
                serde_json::to_value(result).unwrap(),
            ))
            .unwrap()
        }
        "tools/list" => {
            let tools = handlers::tool_definitions();
            let result = ToolsListResult { tools };
            serde_json::to_value(JsonRpcResponse::new(
                id,
                serde_json::to_value(result).unwrap(),
            ))
            .unwrap()
        }
        "tools/call" => {
            let params = match params {
                Some(p) => p,
                None => {
                    let result = ToolCallResult::error("Missing params".to_string());
                    return serde_json::to_value(JsonRpcResponse::new(
                        id,
                        serde_json::to_value(result).unwrap(),
                    ))
                    .unwrap();
                }
            };

            let tool_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Object(serde_json::Map::new()));

            let result = handlers::dispatch_tool(ctx, tool_name, &arguments);

            serde_json::to_value(JsonRpcResponse::new(
                id,
                serde_json::to_value(result).unwrap(),
            ))
            .unwrap()
        }
        "ping" => {
            serde_json::to_value(JsonRpcResponse::new(id, json!({}))).unwrap()
        }
        _ => {
            serde_json::to_value(JsonRpcErrorResponse::new(
                id,
                -32601,
                format!("Method not found: {}", method),
            ))
            .unwrap()
        }
    }
}

use serde_json::json;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_ctx() -> HandlerContext {
        let index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec![],
            index: HashMap::new(),
            total_tokens: 0,
            extensions: vec![],
            file_token_counts: vec![],
            forward: None,
            path_to_id: None,
        };
        HandlerContext {
            index: Arc::new(RwLock::new(index)),
            def_index: None,
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),
        }
    }

    #[test]
    fn test_handle_initialize() {
        let ctx = make_ctx();
        let result = handle_request(&ctx, "initialize", &None, json!(1));
        assert_eq!(result["jsonrpc"], "2.0");
        assert_eq!(result["id"], 1);
        assert_eq!(result["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(result["result"]["serverInfo"]["name"], "search-index");
    }

    #[test]
    fn test_handle_tools_list() {
        let ctx = make_ctx();
        let result = handle_request(&ctx, "tools/list", &None, json!(2));
        assert_eq!(result["jsonrpc"], "2.0");
        assert_eq!(result["id"], 2);
        let tools = result["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 7);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"search_grep"));
        assert!(names.contains(&"search_find"));
        assert!(names.contains(&"search_fast"));
        assert!(names.contains(&"search_info"));
        assert!(names.contains(&"search_reindex"));
        assert!(names.contains(&"search_definitions"));
    }

    #[test]
    fn test_handle_tools_call_grep() {
        let ctx = make_ctx();
        let params = json!({
            "name": "search_grep",
            "arguments": { "terms": "HttpClient" }
        });
        let result = handle_request(&ctx, "tools/call", &Some(params), json!(3));
        assert_eq!(result["jsonrpc"], "2.0");
        assert_eq!(result["id"], 3);
        // Should have content array
        let content = result["result"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }

    #[test]
    fn test_handle_unknown_method() {
        let ctx = make_ctx();
        let result = handle_request(&ctx, "unknown/method", &None, json!(99));
        assert_eq!(result["jsonrpc"], "2.0");
        assert_eq!(result["id"], 99);
        assert!(result["error"]["message"].as_str().unwrap().contains("Method not found"));
        assert_eq!(result["error"]["code"], -32601);
    }

    #[test]
    fn test_handle_ping() {
        let ctx = make_ctx();
        let result = handle_request(&ctx, "ping", &None, json!(42));
        assert_eq!(result["jsonrpc"], "2.0");
        assert_eq!(result["id"], 42);
        assert!(result["result"].is_object());
    }

    #[test]
    fn test_handle_tools_call_missing_params() {
        let ctx = make_ctx();
        let result = handle_request(&ctx, "tools/call", &None, json!(5));
        assert_eq!(result["result"]["isError"], true);
        assert!(result["result"]["content"][0]["text"].as_str().unwrap().contains("Missing params"));
    }
}