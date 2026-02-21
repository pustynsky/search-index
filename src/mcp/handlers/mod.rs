//! MCP tool handlers — dispatches tool calls to specialized handler modules.

mod callers;
mod definitions;
mod fast;
mod find;
mod git;
mod grep;
pub(crate) mod utils;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use serde_json::{json, Value};
use tracing::{info, warn};

use crate::mcp::protocol::{ToolCallResult, ToolDefinition};
use crate::{
    build_content_index, clean_path, cmd_info_json,
    save_content_index, ContentIndex, ContentIndexArgs,
};
use crate::definitions::DefinitionIndex;
use crate::git::cache::GitHistoryCache;

// Re-export for use by tests (crate-internal only)
#[cfg(test)]
pub(crate) use self::callers::find_containing_method;
#[cfg(test)]
pub(crate) use self::callers::resolve_call_site;

/// Return all tool definitions for tools/list
pub fn tool_definitions() -> Vec<ToolDefinition> {
    let mut tools = vec![
        ToolDefinition {
            name: "search_grep".to_string(),
            description: "Search file contents using an inverted index with TF-IDF ranking. LANGUAGE-AGNOSTIC: works with any text file (C#, Rust, Python, JS/TS, XML, JSON, config, etc.). Supports exact tokens, multi-term OR/AND, regex, phrase search, substring search, and exclusion filters. Results ranked by relevance. Index stays in memory for instant subsequent queries (~0.001s). Substring search is ON by default. Large results are auto-truncated to ~16KB (~4K tokens). Use countOnly=true or narrow with dir/ext/excludeDir for focused results.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "terms": {
                        "type": "string",
                        "description": "Search terms. Comma-separated for multi-term OR/AND."
                    },
                    "dir": {
                        "type": "string",
                        "description": "Directory to search (default: server's --dir)"
                    },
                    "ext": {
                        "type": "string",
                        "description": "File extension filter, comma-separated (default: all indexed)"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["or", "and"],
                        "description": "Multi-term mode: 'or' = ANY term (default), 'and' = ALL terms."
                    },
                    "regex": {
                        "type": "boolean",
                        "description": "Treat as regex pattern (default: false)"
                    },
                    "phrase": {
                        "type": "boolean",
                        "description": "Exact phrase match (default: false)"
                    },
                    "showLines": {
                        "type": "boolean",
                        "description": "Include matching source lines in results (default: false)"
                    },
                    "contextLines": {
                        "type": "integer",
                        "description": "Context lines before/after each match, requires showLines (default: 0)"
                    },
                    "maxResults": {
                        "type": "integer",
                        "description": "Max results (0=unlimited, default: 50)"
                    },
                    "excludeDir": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Directory names to exclude"
                    },
                    "exclude": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "File path substrings to exclude"
                    },
                    "countOnly": {
                        "type": "boolean",
                        "description": "Return counts only (default: false)"
                    },
                    "substring": {
                        "type": "boolean",
                        "description": "Match within tokens (default: true). Auto-disabled for regex/phrase."
                    }
                },
                "required": ["terms"]
            }),
        },
        ToolDefinition {
            name: "search_find".to_string(),
            description: "[SLOW — USE search_fast INSTEAD] Search for files by name using live filesystem walk. This is 90x+ slower than search_fast (~3s vs ~35ms). Only use when: (1) no file name index exists, or (2) you need to search outside the indexed directory. For all normal file lookups, use search_fast.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "File name pattern to search for"
                    },
                    "dir": { "type": "string", "description": "Root directory to search" },
                    "ext": { "type": "string", "description": "Filter by extension" },
                    "contents": {
                        "type": "boolean",
                        "description": "Search file contents instead of names"
                    },
                    "regex": { "type": "boolean", "description": "Treat pattern as regex" },
                    "ignoreCase": {
                        "type": "boolean",
                        "description": "Case-insensitive search"
                    },
                    "maxDepth": { "type": "integer", "description": "Max directory depth" },
                    "countOnly": { "type": "boolean", "description": "Return count only" }
                },
                "required": ["pattern"]
            }),
        },
        ToolDefinition {
            name: "search_fast".to_string(),
            description: "PREFERRED file lookup tool — searches pre-built file name index. 90x+ faster than search_find (~35ms vs ~3s for 100K files). Auto-builds index if not present. Supports comma-separated patterns for multi-file lookup (OR logic). Example: pattern='UserService,OrderProcessor' finds files whose name contains ANY of the terms. Always use this instead of search_find for file name lookups.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "File name pattern. Comma-separated for multi-term OR." },
                    "dir": { "type": "string", "description": "Directory to search" },
                    "ext": { "type": "string", "description": "Filter by extension" },
                    "regex": { "type": "boolean", "description": "Treat as regex" },
                    "ignoreCase": { "type": "boolean", "description": "Case-insensitive" },
                    "dirsOnly": { "type": "boolean", "description": "Show only directories" },
                    "filesOnly": { "type": "boolean", "description": "Show only files" },
                    "countOnly": { "type": "boolean", "description": "Count only" }
                },
                "required": ["pattern"]
            }),
        },
        ToolDefinition {
            name: "search_info".to_string(),
            description: "Show all existing indexes with their status, sizes, and age.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "search_reindex".to_string(),
            description: "Force rebuild the content index and reload it into the server's in-memory cache. Useful after many file changes or when --watch is not enabled. The rebuilt index replaces the current in-memory index immediately.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "dir": { "type": "string", "description": "Directory to reindex" },
                    "ext": {
                        "type": "string",
                        "description": "File extensions (comma-separated)"
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "search_reindex_definitions".to_string(),
            description: "Force rebuild the AST definition index (tree-sitter) and reload it into the server's in-memory cache. Returns build metrics: files parsed, definitions extracted, call sites, codeStatsEntries (methods with complexity metrics), parse errors, build time, and index size. After rebuild, code stats are available for includeCodeStats/sortBy/min* queries. Requires server started with --definitions flag.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "dir": { "type": "string", "description": "Directory to reindex (default: server's --dir)" },
                    "ext": {
                        "type": "string",
                        "description": "File extensions to parse, comma-separated (default: server's --ext)"
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "search_definitions".to_string(),
            description: "Search code definitions -- classes, interfaces, methods, properties, enums. Uses pre-built tree-sitter AST index for instant results (~0.001s). LANGUAGE-SPECIFIC: Supports C# and TypeScript/TSX (tree-sitter grammars). SQL parser retained but disabled. Requires server started with --definitions flag. Supports 'containsLine' to find which method/class contains a given line number (no more manual read_file!). Supports 'includeBody' to return actual source code inline, eliminating read_file calls.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name to search (substring). Comma-separated for multi-term OR."
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["class", "interface", "method", "property", "field", "enum", "struct", "record", "constructor", "delegate", "event", "enumMember", "function", "typeAlias", "variable", "storedProcedure", "table", "view", "sqlFunction", "userDefinedType", "column", "sqlIndex"],
                        "description": "Filter by definition kind (see enum for valid values)."
                    },
                    "attribute": {
                        "type": "string",
                        "description": "Filter by C# attribute name."
                    },
                    "baseType": {
                        "type": "string",
                        "description": "Filter by base type or implemented interface."
                    },
                    "file": {
                        "type": "string",
                        "description": "Filter by file path substring."
                    },
                    "parent": {
                        "type": "string",
                        "description": "Filter by parent/containing class name."
                    },
                    "containsLine": {
                        "type": "integer",
                        "description": "Find definition(s) containing this line number. Returns innermost method + parent class. Requires 'file' parameter."
                    },
                    "regex": {
                        "type": "boolean",
                        "description": "Treat name as regex pattern (default: false)."
                    },
                    "maxResults": {
                        "type": "integer",
                        "description": "Max results (default: 100, 0=unlimited)"
                    },
                    "excludeDir": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Directory names to exclude"
                    },
                    "includeBody": {
                        "type": "boolean",
                        "description": "Include source code body in results. Use maxBodyLines to control size. (default: false)"
                    },
                    "maxBodyLines": {
                        "type": "integer",
                        "description": "Max source lines per definition when includeBody=true (default: 100, 0=unlimited)"
                    },
                    "maxTotalBodyLines": {
                        "type": "integer",
                        "description": "Max total body lines across all results (default: 500, 0=unlimited)"
                    },
                    "audit": {
                        "type": "boolean",
                        "description": "Return index coverage report instead of search results. (default: false)"
                    },
                    "auditMinBytes": {
                        "type": "integer",
                        "description": "Min file size to flag as suspicious in audit (default: 500)"
                    },
                    "includeCodeStats": {
                        "type": "boolean",
                        "description": "Include complexity metrics (cyclomatic, cognitive, nesting, params, returns, calls, lambdas). Auto-enabled by sortBy/min*. (default: false)"
                    },
                    "sortBy": {
                        "type": "string",
                        "enum": ["cyclomaticComplexity", "cognitiveComplexity", "maxNestingDepth", "paramCount", "returnCount", "callCount", "lambdaCount", "lines"],
                        "description": "Sort by metric descending (worst first). Auto-enables includeCodeStats."
                    },
                    "minComplexity": {
                        "type": "integer",
                        "description": "Min cyclomatic complexity. Auto-enables includeCodeStats. Multiple min* combine with AND."
                    },
                    "minCognitive": {
                        "type": "integer",
                        "description": "Min cognitive complexity. Auto-enables includeCodeStats."
                    },
                    "minNesting": {
                        "type": "integer",
                        "description": "Min nesting depth. Auto-enables includeCodeStats."
                    },
                    "minParams": {
                        "type": "integer",
                        "description": "Min parameter count. Auto-enables includeCodeStats."
                    },
                    "minReturns": {
                        "type": "integer",
                        "description": "Min return/throw count. Auto-enables includeCodeStats."
                    },
                    "minCalls": {
                        "type": "integer",
                        "description": "Min call count (fan-out). Auto-enables includeCodeStats."
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "search_callers".to_string(),
            description: "RECOMMENDED for call chain analysis -- find all callers of a method and build a call tree (up or down) in a SINGLE sub-millisecond request. Supports C# and TypeScript/TSX. DI-aware. Returns a hierarchical call tree with method signatures, file paths, and line numbers. Always specify the 'class' parameter to avoid mixing callers from unrelated classes. Requires server started with --definitions flag. Limitation: calls through local variables (e.g., `var x = service.GetFoo(); x.Bar()`) may not be detected because the tool uses AST parsing without type inference. DI-injected fields, `this`/`base` calls, and direct receiver calls are fully supported.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "method": {
                        "type": "string",
                        "description": "Method name to find callers/callees for."
                    },
                    "class": {
                        "type": "string",
                        "description": "STRONGLY RECOMMENDED: Parent class name to scope the search. Without this, callers of ALL methods with this name across the entire codebase are found, which may mix results from unrelated classes and produce misleading call trees. Always specify when you know the containing class. DI-aware: automatically includes callers that use the interface (e.g., class='UserService' also finds callers using IUserService)."
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Max recursion depth (default: 3, max: 10)"
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["up", "down"],
                        "description": "'up' = callers (default), 'down' = callees."
                    },
                    "ext": {
                        "type": "string",
                        "description": "File extension filter (default: server's --ext)"
                    },
                    "excludeDir": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Directory names to exclude"
                    },
                    "excludeFile": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "File path substrings to exclude"
                    },
                    "maxCallersPerLevel": {
                        "type": "integer",
                        "description": "Max callers per tree node (default: 10)"
                    },
                    "maxTotalNodes": {
                        "type": "integer",
                        "description": "Max total nodes in call tree (default: 200)"
                    },
                    "resolveInterfaces": {
                        "type": "boolean",
                        "description": "Auto-resolve interface methods to implementations (default: true)"
                    }
                },
                "required": ["method"]
            }),
        },
        ToolDefinition {
            name: "search_help".to_string(),
            description: "Show best practices and usage tips for search-index tools. Call this when unsure which tool to use or how to optimize queries. Returns a concise guide with tool selection priorities, performance tiers, and common pitfalls.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    ];

    // Git history tools (always available)
    tools.extend(git::git_tool_definitions());

    tools
}

/// Context for tool handlers -- shared state
pub struct HandlerContext {
    pub index: Arc<RwLock<ContentIndex>>,
    pub def_index: Option<Arc<RwLock<DefinitionIndex>>>,
    pub server_dir: String,
    pub server_ext: String,
    pub metrics: bool,
    /// Base directory for index file storage.
    /// Production: `index_dir()` (`%LOCALAPPDATA%/search-index`).
    /// Tests: test-local temp directory (prevents orphan files).
    pub index_base: PathBuf,
    /// Maximum response size in bytes before truncation kicks in. 0 = no limit.
    pub max_response_bytes: usize,
    /// Whether the content index has been fully built/loaded.
    /// Tools return a "building" message when false.
    pub content_ready: Arc<AtomicBool>,
    /// Whether the definition index has been fully built/loaded.
    /// Tools return a "building" message when false.
    pub def_ready: Arc<AtomicBool>,
    /// Git history cache — populated by background thread (PR 2c).
    /// `None` until cache is built; queries fall back to CLI.
    pub git_cache: Arc<RwLock<Option<GitHistoryCache>>>,
    /// Fast readiness check for git cache (avoids RwLock contention).
    pub git_cache_ready: Arc<AtomicBool>,
    /// Current checked-out branch name (detected at server startup).
    /// Used to inject branchWarning into index-based tool responses.
    pub current_branch: Option<String>,
}

/// Message returned when the content index is still building in background.
const INDEX_BUILDING_MSG: &str =
    "Content index is currently being built in the background. Please retry in a few seconds.";

/// Message returned when the definition index is still building in background.
const DEF_INDEX_BUILDING_MSG: &str =
    "Definition index is currently being built in the background. Please retry in a few seconds.";

/// Message returned when search_reindex is called while a background build is in progress.
const ALREADY_BUILDING_MSG: &str =
    "Index is already being built in the background. Please wait for it to finish.";

/// Returns true when a tool requires the content index to be ready.
fn requires_content_index(tool_name: &str) -> bool {
    matches!(tool_name, "search_grep" | "search_fast" | "search_reindex")
}

/// Returns true when a tool requires the definition index to be ready.
fn requires_def_index(tool_name: &str) -> bool {
    matches!(tool_name, "search_definitions" | "search_callers" | "search_reindex_definitions")
}

/// Dispatch a tool call to the right handler.
/// When `ctx.metrics` is true, injects performance metrics into the response summary.
pub fn dispatch_tool(
    ctx: &HandlerContext,
    tool_name: &str,
    arguments: &Value,
) -> ToolCallResult {
    let dispatch_start = Instant::now();

    // Check readiness: if the required index is still building, return early
    if requires_content_index(tool_name) && !ctx.content_ready.load(Ordering::Acquire) {
        if tool_name == "search_reindex" {
            return ToolCallResult::error(ALREADY_BUILDING_MSG.to_string());
        }
        return ToolCallResult::error(INDEX_BUILDING_MSG.to_string());
    }
    if requires_def_index(tool_name) && !ctx.def_ready.load(Ordering::Acquire) {
        if tool_name == "search_reindex_definitions" {
            return ToolCallResult::error(ALREADY_BUILDING_MSG.to_string());
        }
        return ToolCallResult::error(DEF_INDEX_BUILDING_MSG.to_string());
    }

    let result = match tool_name {
        "search_grep" => grep::handle_search_grep(ctx, arguments),
        "search_find" => find::handle_search_find(ctx, arguments),
        "search_fast" => fast::handle_search_fast(ctx, arguments),
        "search_info" => handle_search_info(),
        "search_reindex" => handle_search_reindex(ctx, arguments),
        "search_reindex_definitions" => handle_search_reindex_definitions(ctx, arguments),
        "search_definitions" => definitions::handle_search_definitions(ctx, arguments),
        "search_callers" => callers::handle_search_callers(ctx, arguments),
        "search_help" => handle_search_help(),
        // Git history tools
        "search_git_history" | "search_git_diff" | "search_git_authors" | "search_git_activity" | "search_git_blame" | "search_branch_status" | "search_git_pickaxe" => {
            git::dispatch_git_tool(ctx, tool_name, arguments)
        }
        _ => return ToolCallResult::error(format!("Unknown tool: {}", tool_name)),
    };

    if result.is_error {
        return result;
    }

    if ctx.metrics {
        // inject_metrics also calls truncate_large_response internally
        utils::inject_metrics(result, ctx, dispatch_start)
    } else {
        // Even without metrics, apply response size truncation
        utils::truncate_response_if_needed(result, ctx.max_response_bytes)
    }
}

// ─── Small inline handlers ──────────────────────────────────────────

fn handle_search_help() -> ToolCallResult {
    let help = crate::tips::render_json();
    ToolCallResult::success(serde_json::to_string_pretty(&help).unwrap())
}

fn handle_search_info() -> ToolCallResult {
    let info = cmd_info_json();
    ToolCallResult::success(serde_json::to_string(&info).unwrap())
}

fn handle_search_reindex(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
    let dir = args.get("dir").and_then(|v| v.as_str()).unwrap_or(&ctx.server_dir);
    let ext = args.get("ext").and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| ctx.server_ext.clone());

    // Check dir matches server dir
    let requested = std::fs::canonicalize(dir)
        .map(|p| clean_path(&p.to_string_lossy()))
        .unwrap_or_else(|_| dir.to_string());
    let server = std::fs::canonicalize(&ctx.server_dir)
        .map(|p| clean_path(&p.to_string_lossy()))
        .unwrap_or_else(|_| ctx.server_dir.clone());
    if !requested.eq_ignore_ascii_case(&server) {
        return ToolCallResult::error(format!(
            "Server started with --dir {}. For other directories, start another server instance or use CLI.",
            ctx.server_dir
        ));
    }

    info!(dir = %dir, ext = %ext, "Rebuilding content index");
    let start = Instant::now();

    let new_index = build_content_index(&ContentIndexArgs {
        dir: dir.to_string(),
        ext: ext.clone(),
        max_age_hours: 24,
        hidden: false,
        no_ignore: false,
        threads: 0,
        min_token_len: 2,
    });

    // Save to disk
    if let Err(e) = save_content_index(&new_index, &ctx.index_base) {
        warn!(error = %e, "Failed to save reindexed content to disk");
    }

    let file_count = new_index.files.len();
    let token_count = new_index.index.len();

    // Update in-memory cache
    match ctx.index.write() {
        Ok(mut idx) => {
            *idx = new_index;
        }
        Err(e) => return ToolCallResult::error(format!("Failed to update in-memory index: {}", e)),
    }

    let elapsed = start.elapsed();

    let output = json!({
        "status": "ok",
        "files": file_count,
        "uniqueTokens": token_count,
        "rebuildTimeMs": elapsed.as_secs_f64() * 1000.0,
    });

    ToolCallResult::success(serde_json::to_string(&output).unwrap())
}

fn handle_search_reindex_definitions(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
    let def_index_arc = match &ctx.def_index {
        Some(di) => Arc::clone(di),
        None => return ToolCallResult::error(
            "Definition index not available. Start server with --definitions flag.".to_string()
        ),
    };

    let dir = args.get("dir").and_then(|v| v.as_str()).unwrap_or(&ctx.server_dir);
    let ext = args.get("ext").and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| ctx.server_ext.clone());

    // Check dir matches server dir
    let requested = std::fs::canonicalize(dir)
        .map(|p| clean_path(&p.to_string_lossy()))
        .unwrap_or_else(|_| dir.to_string());
    let server = std::fs::canonicalize(&ctx.server_dir)
        .map(|p| clean_path(&p.to_string_lossy()))
        .unwrap_or_else(|_| ctx.server_dir.clone());
    if !requested.eq_ignore_ascii_case(&server) {
        return ToolCallResult::error(format!(
            "Server started with --dir {}. For other directories, start another server instance or use CLI.",
            ctx.server_dir
        ));
    }

    info!(dir = %dir, ext = %ext, "Rebuilding definition index");
    let start = Instant::now();

    let new_index = crate::definitions::build_definition_index(&crate::definitions::DefIndexArgs {
        dir: dir.to_string(),
        ext: ext.clone(),
        threads: 0,
    });

    // Save to disk
    if let Err(e) = crate::definitions::save_definition_index(&new_index, &ctx.index_base) {
        warn!(error = %e, "Failed to save definition index to disk");
    }

    let file_count = new_index.files.len();
    let def_count = new_index.definitions.len();
    let call_site_count: usize = new_index.method_calls.values().map(|v| v.len()).sum();
    let code_stats_count = new_index.code_stats.len();

    // Compute index size without allocating (uses bincode::serialized_size)
    let size_mb = bincode::serialized_size(&new_index)
        .map(|size| size as f64 / 1_048_576.0)
        .unwrap_or(0.0);

    // Update in-memory cache
    match def_index_arc.write() {
        Ok(mut idx) => {
            *idx = new_index;
        }
        Err(e) => return ToolCallResult::error(format!("Failed to update in-memory definition index: {}", e)),
    }

    let elapsed = start.elapsed();

    let output = json!({
        "status": "ok",
        "files": file_count,
        "definitions": def_count,
        "callSites": call_site_count,
        "codeStatsEntries": code_stats_count,
        "sizeMb": (size_mb * 10.0).round() / 10.0,
        "rebuildTimeMs": elapsed.as_secs_f64() * 1000.0,
    });

    ToolCallResult::success(serde_json::to_string(&output).unwrap())
}

// ─── Tests ──────────────────────────────────────────────────────────
// Tests remain in the original handlers_tests.rs file to avoid
// duplicating ~3000 lines. They use `use super::*` to access
// all re-exported symbols.

#[cfg(test)]
mod handlers_test_utils;

#[cfg(test)]
#[path = "handlers_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "handlers_tests_csharp.rs"]
mod tests_csharp;

#[cfg(test)]
#[path = "handlers_tests_typescript.rs"]
mod tests_typescript;