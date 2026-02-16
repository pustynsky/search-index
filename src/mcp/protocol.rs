use serde::{Deserialize, Serialize};
use serde_json::Value;

// ─── JSON-RPC 2.0 base types ────────────────────────────────────────

/// Incoming JSON-RPC request (may be a notification if id is None)
#[derive(Deserialize, Debug)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// Outgoing JSON-RPC response
#[derive(Serialize, Debug)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    pub result: Value,
}

/// Outgoing JSON-RPC error response
#[derive(Serialize, Debug)]
pub struct JsonRpcErrorResponse {
    pub jsonrpc: String,
    pub id: Value,
    pub error: JsonRpcError,
}

#[derive(Serialize, Debug)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

// ─── MCP Initialize types ───────────────────────────────────────────

#[derive(Serialize, Debug)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
    /// MCP server-level instructions for LLM clients.
    /// Provides best practices and tool selection guidance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct ServerCapabilities {
    pub tools: ToolsCapability,
}

#[derive(Serialize, Debug)]
pub struct ToolsCapability {
    #[serde(rename = "listChanged")]
    pub list_changed: bool,
}

#[derive(Serialize, Debug)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

// ─── MCP Tools types ────────────────────────────────────────────────

#[derive(Serialize, Debug)]
pub struct ToolsListResult {
    pub tools: Vec<ToolDefinition>,
}

#[derive(Serialize, Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// MCP tool call result content
#[derive(Serialize, Debug)]
pub struct ToolCallResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
}

#[derive(Serialize, Debug)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

// ─── Helper constructors ────────────────────────────────────────────

impl JsonRpcResponse {
    pub fn new(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result,
        }
    }
}

impl JsonRpcErrorResponse {
    pub fn new(id: Value, code: i64, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            error: JsonRpcError { code, message },
        }
    }
}

impl ToolCallResult {
    pub fn success(text: String) -> Self {
        Self {
            content: vec![ToolContent {
                content_type: "text".to_string(),
                text,
            }],
            is_error: false,
        }
    }

    pub fn error(text: String) -> Self {
        Self {
            content: vec![ToolContent {
                content_type: "text".to_string(),
                text,
            }],
            is_error: true,
        }
    }
}

impl InitializeResult {
    pub fn new() -> Self {
        Self {
            protocol_version: "2025-03-26".to_string(),
            capabilities: ServerCapabilities {
                tools: ToolsCapability {
                    list_changed: false,
                },
            },
            server_info: ServerInfo {
                name: "search-index".to_string(),
                version: "0.3.0".to_string(),
            },
            instructions: Some(Self::instructions_text().to_string()),
        }
    }

    /// Server-level best practices for LLM tool selection.
    /// These address gaps that are NOT discoverable from individual tool descriptions alone.
    fn instructions_text() -> &'static str {
        concat!(
            "search-index MCP server — Best Practices for Tool Selection\n",
            "\n",
            "1. FILE LOOKUP: Always use search_fast (indexed, ~35ms) instead of search_find (live filesystem walk, ~3s). ",
            "search_fast is 90x+ faster. Only use search_find when no index exists.\n",
            "\n",
            "2. MULTI-TERM OR: Find all variants of a class in ONE query with comma-separated terms. ",
            "Example: terms='UserService,IUserService,UserServiceFactory', mode='or'. ",
            "Much faster than making 3 separate queries.\n",
            "\n",
            "3. EXCLUDE TEST DIRS: Use excludeDir=['test','Mock','UnitTests'] to get production-only results. ",
            "Works in search_grep, search_definitions, and search_callers. Half the results are often test files.\n",
            "\n",
            "4. SUBSTRING SEARCH: In C#/Java codebases with compound identifiers, use search_grep with substring=true. ",
            "Default exact-token mode will NOT find 'UserService' inside 'DeleteUserServiceCacheEntry'. Fast (~1ms).\n",
            "\n",
            "5. CALL CHAIN TRACING: Use search_callers instead of chaining search_grep + read_file. ",
            "Single sub-millisecond request replaces 7+ sequential calls. Supports direction='up' (who calls this) ",
            "and direction='down' (what does this call). Always specify the class parameter to avoid mixing callers ",
            "from unrelated classes with the same method name.\n",
            "\n",
            "6. STACK TRACE ANALYSIS: Use search_definitions with file='MyFile.cs', containsLine=42 to find which ",
            "method/class contains a given line number. Returns the innermost method and its parent class.\n",
            "\n",
            "7. READING METHOD SOURCE: Use search_definitions with includeBody=true instead of read_file. ",
            "Returns method body inline. Body budgets: maxBodyLines (per def, default 100), maxTotalBodyLines ",
            "(all defs, default 500). Set to 0 for unlimited.\n",
            "\n",
            "8. AND MODE: Use search_grep with mode='and' to find files containing ALL comma-separated terms. ",
            "Example: terms='ServiceProvider,IUserService', mode='and' finds only files with both.\n",
            "\n",
            "9. PHRASE/REGEX: Use phrase=true for exact multi-word match ('new HttpClient'), or regex=true ",
            "for patterns ('I[A-Z]\\w+Cache'). Both are slower (~60-80ms) but precise.\n",
            "\n",
            "10. RECONNAISSANCE: Use search_grep with countOnly=true for quick 'how many files use X?' — ~46 tokens vs 265+.\n",
            "\n",
            "11. TOOL PRIORITY:\n",
            "   - search_callers (call trees up/down, <1ms)\n",
            "   - search_definitions (structural: classes, methods, containsLine, <1ms for baseType/attribute)\n",
            "   - search_grep (content: exact/OR/AND <1ms, substring ~1ms, phrase/regex ~60-80ms)\n",
            "   - search_fast (file name lookup, ~35ms)\n",
            "   - search_find (live walk, ~3s — last resort)\n",
            "\n",
            "Call search_help for a detailed JSON guide with examples.\n",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_initialize_request() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "initialize");
        assert_eq!(req.id, Some(serde_json::json!(1)));
        assert!(req.params.is_some());
    }

    #[test]
    fn test_parse_notification() {
        let json = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "notifications/initialized");
        assert!(req.id.is_none());
    }

    #[test]
    fn test_parse_tools_list_request() {
        let json = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "tools/list");
        assert_eq!(req.id, Some(serde_json::json!(2)));
    }

    #[test]
    fn test_parse_tools_call_request() {
        let json = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"HttpClient","mode":"or"}}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "tools/call");
        let params = req.params.unwrap();
        assert_eq!(params["name"], "search_grep");
        assert_eq!(params["arguments"]["terms"], "HttpClient");
        assert_eq!(params["arguments"]["mode"], "or");
    }

    #[test]
    fn test_initialize_response_format() {
        let result = InitializeResult::new();
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["protocolVersion"], "2025-03-26");
        assert_eq!(json["capabilities"]["tools"]["listChanged"], false);
        assert_eq!(json["serverInfo"]["name"], "search-index");
        assert_eq!(json["serverInfo"]["version"], "0.3.0");
    }

    #[test]
    fn test_initialize_includes_instructions() {
        let result = InitializeResult::new();
        let json = serde_json::to_value(&result).unwrap();
        let instructions = json["instructions"].as_str().unwrap();
        assert!(instructions.contains("search_fast"), "instructions should mention search_fast");
        assert!(instructions.contains("search_find"), "instructions should mention search_find");
        assert!(instructions.contains("substring"), "instructions should mention substring search");
        assert!(instructions.contains("search_callers"), "instructions should mention search_callers");
        assert!(instructions.contains("class"), "instructions should mention class parameter");
        assert!(instructions.contains("includeBody"), "instructions should mention includeBody");
        assert!(instructions.contains("countOnly"), "instructions should mention countOnly");
    }

    #[test]
    fn test_jsonrpc_response_format() {
        let resp = JsonRpcResponse::new(
            serde_json::json!(1),
            serde_json::to_value(InitializeResult::new()).unwrap(),
        );
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert!(parsed["result"]["protocolVersion"].is_string());
    }

    #[test]
    fn test_tool_call_success_result() {
        let result = ToolCallResult::success("hello".to_string());
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["content"][0]["type"], "text");
        assert_eq!(json["content"][0]["text"], "hello");
        // isError should not appear when false
        assert!(json.get("isError").is_none());
    }

    #[test]
    fn test_tool_call_error_result() {
        let result = ToolCallResult::error("something failed".to_string());
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["content"][0]["text"], "something failed");
        assert_eq!(json["isError"], true);
    }

    #[test]
    fn test_jsonrpc_error_response() {
        let resp = JsonRpcErrorResponse::new(
            serde_json::json!(5),
            -32601,
            "Method not found".to_string(),
        );
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 5);
        assert_eq!(json["error"]["code"], -32601);
        assert_eq!(json["error"]["message"], "Method not found");
    }
}