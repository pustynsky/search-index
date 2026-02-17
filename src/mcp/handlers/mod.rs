//! MCP tool handlers — dispatches tool calls to specialized handler modules.

mod callers;
mod definitions;
mod fast;
mod find;
mod grep;
pub(crate) mod utils;

use std::path::PathBuf;
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

// Re-export for use by tests (crate-internal only)
#[cfg(test)]
pub(crate) use self::callers::find_containing_method;
#[cfg(test)]
pub(crate) use self::callers::resolve_call_site;

/// Return all tool definitions for tools/list
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "search_grep".to_string(),
            description: "Search file contents using an inverted index with TF-IDF ranking. LANGUAGE-AGNOSTIC: works with any text file (C#, Rust, Python, JS/TS, XML, JSON, config, etc.) — just specify the extension via ext parameter or server --ext flag. Supports exact tokens, multi-term OR/AND, regex, phrase search, substring search, and exclusion filters. Results ranked by relevance. Index stays in memory for instant subsequent queries (~0.001s). IMPORTANT: When searching for all usages of a class/interface, use multi-term OR search to find ALL naming variants in ONE query. Example: to find all usages of MyClass, search for 'MyClass,IMyClass,MyClassFactory' with mode='or'. This is much faster than making separate queries for each variant. Comma-separated terms with mode='or' finds files containing ANY of the terms; mode='and' finds files containing ALL terms. SUBSTRING SEARCH IS ON BY DEFAULT: compound identifiers like 'IUserService', 'm_userService', 'DeleteUserServiceCacheEntry' are automatically found when searching for 'UserService'. Uses a trigram index (~1ms). Set substring=false for exact-token-only matching. Auto-disabled when regex or phrase is used. RESPONSE TRUNCATION: Large results are auto-truncated to ~16KB (~4K tokens) to protect LLM context. If summary.responseTruncated=true, narrow your query with dir/ext/excludeDir or use countOnly=true. Server flag --max-response-kb adjusts the limit (0=unlimited).".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "terms": {
                        "type": "string",
                        "description": "Search terms. Comma-separated for multi-term search. Single token: 'HttpClient'. Multi-term OR/AND: 'HttpClient,ILogger,Task' (finds files with ANY term when mode='or', or ALL terms when mode='and'). Always use comma-separated multi-term OR search when looking for all usages of a class -- include the class name, its interface, and related types in one query. Phrase (use with phrase=true): 'new HttpClient'. Regex (use with regex=true): 'I.*Cache'"
                    },
                    "dir": {
                        "type": "string",
                        "description": "Directory to search. Can be the server's --dir or any subdirectory of it to narrow results. (default: server's --dir)"
                    },
                    "ext": {
                        "type": "string",
                        "description": "File extension filter, e.g. 'cs', 'csproj', 'xml', 'config' (default: server's --ext)"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["or", "and"],
                        "description": "For multi-term search: 'or' = files containing ANY of the comma-separated terms (default), 'and' = files containing ALL terms. Use 'or' mode when searching for all usages/variants of a class (e.g., 'MyCache,IMyCache,MyCacheFactory'). Use 'and' mode when searching for files that use multiple specific types together."
                    },
                    "regex": {
                        "type": "boolean",
                        "description": "Treat terms as regex pattern (default: false)"
                    },
                    "phrase": {
                        "type": "boolean",
                        "description": "Treat input as exact phrase to find (default: false)"
                    },
                    "showLines": {
                        "type": "boolean",
                        "description": "Include source code from matching files. Returns groups of consecutive lines, each with startLine (1-based), lines (string array), and matchIndices (0-based indices of matching lines within the group). Groups are separated when there are gaps in line numbers. (default: false)"
                    },
                    "contextLines": {
                        "type": "integer",
                        "description": "Number of context lines to show before AND after each matching line (requires showLines=true). Works like grep -C. Example: contextLines=5 shows 5 lines before and 5 lines after each match, giving you enough surrounding code to understand the usage. (default: 0)"
                    },
                    "maxResults": {
                        "type": "integer",
                        "description": "Maximum number of results to return (0 = unlimited, default: 50)"
                    },
                    "excludeDir": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Directory names to exclude from results, e.g. ['test', 'E2E']"
                    },
                    "exclude": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "File path substring patterns to exclude, e.g. ['Mock', 'Test']"
                    },
                    "countOnly": {
                        "type": "boolean",
                        "description": "Return only file count and occurrence count (default: false)"
                    },
                    "substring": {
                        "type": "boolean",
                        "description": "Treat each term as a substring to match within tokens (default: true). Finds compound C# identifiers automatically: 'UserService' matches 'IUserService', 'm_userService', 'UserServiceFactory'. Uses trigram index (~1ms). Set to false for exact-token-only matching. Auto-disabled when regex or phrase is used."
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
                    "pattern": { "type": "string", "description": "File name pattern. Comma-separated for multi-term OR search: 'ClassA,ClassB,ClassC' finds files matching ANY term. Single term: 'UserService' finds files containing 'UserService'." },
                    "dir": {
                        "type": "string",
                        "description": "Directory whose index to search"
                    },
                    "ext": { "type": "string", "description": "Filter by extension" },
                    "regex": { "type": "boolean", "description": "Treat pattern as regex" },
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
            description: "Force rebuild the AST definition index (tree-sitter) and reload it into the server's in-memory cache. Returns build metrics: files parsed, definitions extracted, call sites, parse errors, build time, and index size. Requires server started with --definitions flag.".to_string(),
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
            description: "Search code definitions -- classes, interfaces, methods, properties, enums. Uses pre-built tree-sitter AST index for instant results (~0.001s). LANGUAGE-SPECIFIC: currently C# only (tree-sitter grammar required; SQL parser retained but disabled). Requires server started with --definitions flag. Supports 'containsLine' to find which method/class contains a given line number (no more manual read_file!). Supports 'includeBody' to return actual source code inline, eliminating read_file calls.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Definition name to search for. Supports substring match. Comma-separated for multi-term OR search. Example: 'UserService' or 'IUser,UserService'"
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["class", "interface", "method", "property", "field", "enum", "struct", "record", "constructor", "delegate", "event", "enumMember", "storedProcedure", "table", "view", "sqlFunction", "userDefinedType", "column", "sqlIndex"],
                        "description": "Filter by definition kind. C# kinds: class, interface, method, property, field, enum, struct, record, constructor, delegate, event. SQL kinds: storedProcedure, table, view, sqlFunction, userDefinedType."
                    },
                    "attribute": {
                        "type": "string",
                        "description": "Filter by C# attribute name. Example: 'ApiController', 'Authorize', 'ServiceProvider'"
                    },
                    "baseType": {
                        "type": "string",
                        "description": "Filter by base type or implemented interface. Example: 'ControllerBase', 'IUserService'"
                    },
                    "file": {
                        "type": "string",
                        "description": "Filter by file path substring. Example: 'Controllers', 'Services'"
                    },
                    "parent": {
                        "type": "string",
                        "description": "Filter by parent/containing class name. Example: 'UserService' to find all members of that class."
                    },
                    "containsLine": {
                        "type": "integer",
                        "description": "Find the definition(s) that contain this line number. Returns the innermost method/property and its parent class. Must be used with 'file' parameter. Example: file='UserService.cs', containsLine=42 -> returns GetUserAsync (lines 35-50), parent: UserService"
                    },
                    "regex": {
                        "type": "boolean",
                        "description": "Treat name as regex pattern (default: false). Example: name='I.*Cache' with regex=true"
                    },
                    "maxResults": {
                        "type": "integer",
                        "description": "Max results to return (default: 100, 0 = unlimited)"
                    },
                    "excludeDir": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Directory names to exclude from results"
                    },
                    "includeBody": {
                        "type": "boolean",
                        "description": "Include source code body of each definition in the results. Reads the actual file and returns lines from line_start to line_end for each definition. Combine with maxBodyLines to control output size. (default: false)"
                    },
                    "maxBodyLines": {
                        "type": "integer",
                        "description": "Maximum number of source code lines to include per definition when includeBody=true. If a definition has more lines than this limit, only the first maxBodyLines lines are returned, with a 'truncated' flag. (default: 100, 0 = unlimited)"
                    },
                    "maxTotalBodyLines": {
                        "type": "integer",
                        "description": "Maximum total lines of body content across ALL returned definitions. When budget is exhausted, remaining definitions are returned without body (with 'bodyOmitted' marker). Prevents output explosion when many definitions match. (default: 500, 0 = unlimited)"
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "search_callers".to_string(),
            description: "RECOMMENDED for call chain analysis — find all callers of a method and build a call tree (up or down) in a SINGLE sub-millisecond request. LANGUAGE-SPECIFIC: currently C# only (requires tree-sitter AST definition index). Replaces 7+ sequential search_grep + read_file calls. Combines grep index with AST definition index. Returns a hierarchical call tree with method signatures, file paths, and line numbers. IMPORTANT: Always specify the 'class' parameter when you know the containing class — without it, results may mix callers from unrelated classes that have a method with the same name. DI-aware: class='UserService' automatically includes callers using IUserService. Requires server started with --definitions flag.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "method": {
                        "type": "string",
                        "description": "Method name to find callers/callees for. Example: 'GetUserAsync'"
                    },
                    "class": {
                        "type": "string",
                        "description": "STRONGLY RECOMMENDED: Parent class name to scope the search. Without this, callers of ALL methods with this name across the entire codebase are found, which may mix results from unrelated classes and produce misleading call trees. Always specify when you know the containing class. DI-aware: automatically includes callers that use the interface (e.g., class='UserService' also finds callers using IUserService). Example: 'UserService'"
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Maximum recursion depth for the call tree (default: 3, max: 10). Each level finds callers of the callers."
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["up", "down"],
                        "description": "Direction to trace: 'up' = find who calls this method (callers, default), 'down' = find what this method calls (callees). 'up' is most common for tracing entry points."
                    },
                    "ext": {
                        "type": "string",
                        "description": "File extension filter (default: server's --ext). Example: 'cs'"
                    },
                    "excludeDir": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Directory names to exclude from results, e.g. ['test', 'E2E', 'Mock']"
                    },
                    "excludeFile": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "File path substrings to exclude, e.g. ['Test', 'Mock']"
                    },
                    "maxCallersPerLevel": {
                        "type": "integer",
                        "description": "Maximum number of callers to return per tree node (default: 10). Prevents explosion when a method is called from 100+ places."
                    },
                    "maxTotalNodes": {
                        "type": "integer",
                        "description": "Maximum total nodes in the call tree (default: 200). Prevents massive output for heavily-used methods."
                    },
                    "resolveInterfaces": {
                        "type": "boolean",
                        "description": "Auto-resolve interface methods to implementation classes. When tracing callers of IFoo.Bar(), also finds callers of FooImpl.Bar() where FooImpl implements IFoo. (default: true)"
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
    ]
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
}

/// Dispatch a tool call to the right handler.
/// When `ctx.metrics` is true, injects performance metrics into the response summary.
pub fn dispatch_tool(
    ctx: &HandlerContext,
    tool_name: &str,
    arguments: &Value,
) -> ToolCallResult {
    let dispatch_start = Instant::now();

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

    // Compute index size on disk
    let size_mb = bincode::serialize(&new_index)
        .map(|data| data.len() as f64 / 1_048_576.0)
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
#[path = "handlers_tests.rs"]
mod tests;