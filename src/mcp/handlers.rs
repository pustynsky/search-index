use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use serde_json::{json, Value};
use tracing::{info, warn};

use crate::mcp::protocol::{ToolCallResult, ToolDefinition};
use crate::{
    build_content_index, clean_path, cmd_info_json,
    save_content_index, tokenize, ContentIndex, ContentIndexArgs,
};
use crate::index::build_trigram_index;
use search::generate_trigrams;
use crate::definitions::{CallSite, DefinitionEntry, DefinitionIndex, DefinitionKind};

/// Return all tool definitions for tools/list
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "search_grep".to_string(),
            description: "Search file contents using an inverted index with TF-IDF ranking. Supports exact tokens, multi-term OR/AND, regex, phrase search, substring search, and exclusion filters. Results ranked by relevance. Index stays in memory for instant subsequent queries (~0.001s). IMPORTANT: When searching for all usages of a class/interface, use multi-term OR search to find ALL naming variants in ONE query. Example: to find all usages of MyClass, search for 'MyClass,IMyClass,MyClassFactory' with mode='or'. This is much faster than making separate queries for each variant. Comma-separated terms with mode='or' finds files containing ANY of the terms; mode='and' finds files containing ALL terms. Use substring=true to find tokens containing a substring (e.g., 'DatabaseConn' finds 'databaseconnectionfactory').".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "terms": {
                        "type": "string",
                        "description": "Search terms. Comma-separated for multi-term search. Single token: 'HttpClient'. Multi-term OR/AND: 'HttpClient,ILogger,Task' (finds files with ANY term when mode='or', or ALL terms when mode='and'). Always use comma-separated multi-term OR search when looking for all usages of a class â€” include the class name, its interface, and related types in one query. Phrase (use with phrase=true): 'new HttpClient'. Regex (use with regex=true): 'I.*Cache'"
                    },
                    "dir": {
                        "type": "string",
                        "description": "Directory to search (default: server's --dir)"
                    },
                    "ext": {
                        "type": "string",
                        "description": "File extension filter, e.g. 'cs' (default: server's --ext)"
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
                        "description": "Treat each term as a substring to match within tokens (default: false). Example: 'DatabaseConn' with substring=true finds tokens like 'databaseconnectionfactory'. Uses trigram index for fast matching. For queries shorter than 4 chars, a warning is included. Mutually exclusive with 'regex' and 'phrase'."
                    }
                },
                "required": ["terms"]
            }),
        },
        ToolDefinition {
            name: "search_find".to_string(),
            description: "Search for files by name using live filesystem walk. No index needed. âš ï¸ WARNING: This performs a live filesystem walk and may be slow for large directories (10-30s). For instant results, use search_fast with a pre-built file name index.".to_string(),
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
            description: "Search pre-built file name index for instant results. Auto-builds index if not present.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "File name pattern" },
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
                "properties": {}
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
                }
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
                }
            }),
        },
        ToolDefinition {
            name: "search_definitions".to_string(),
            description: "Search C# and SQL code definitions â€” classes, interfaces, methods, properties, enums, stored procedures, tables. Uses pre-built tree-sitter AST index for instant results (~0.001s). Requires server started with --definitions flag. Supports 'containsLine' to find which method/class contains a given line number (no more manual read_file!). Supports 'includeBody' to return actual source code inline, eliminating read_file calls.".to_string(),
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
                        "description": "Find the definition(s) that contain this line number. Returns the innermost method/property and its parent class. Must be used with 'file' parameter. Example: file='UserService.cs', containsLine=42 â†’ returns GetUserAsync (lines 35-50), parent: UserService"
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
            description: "Find all callers of a method and build a call tree (up or down). Combines grep index (to find where a method name appears) with AST definition index (to determine which method/class contains each call site). Returns a hierarchical call tree. This is the most powerful tool for tracing call chains â€” replaces 7+ sequential search_grep + read_file calls with a single request. Requires server started with --definitions flag.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "method": {
                        "type": "string",
                        "description": "Method name to find callers/callees for. Example: 'GetUserAsync'"
                    },
                    "class": {
                        "type": "string",
                        "description": "Parent class name to scope the search. Without this, callers of ALL methods with this name are found (may mix results from unrelated classes). With class specified, only callers that reference this class or its interfaces are returned. DI-aware: automatically includes callers that use the interface (e.g., class='UserService' also finds callers using IUserService). Example: 'UserService'"
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
    ]
}

/// Context for tool handlers â€” shared state
pub struct HandlerContext {
    pub index: Arc<RwLock<ContentIndex>>,
    pub def_index: Option<Arc<RwLock<DefinitionIndex>>>,
    pub server_dir: String,
    pub server_ext: String,
}

/// Dispatch a tool call to the right handler
pub fn dispatch_tool(
    ctx: &HandlerContext,
    tool_name: &str,
    arguments: &Value,
) -> ToolCallResult {
    match tool_name {
        "search_grep" => handle_search_grep(ctx, arguments),
        "search_find" => handle_search_find(ctx, arguments),
        "search_fast" => handle_search_fast(ctx, arguments),
        "search_info" => handle_search_info(),
        "search_reindex" => handle_search_reindex(ctx, arguments),
        "search_reindex_definitions" => handle_search_reindex_definitions(ctx, arguments),
        "search_definitions" => handle_search_definitions(ctx, arguments),
        "search_callers" => handle_search_callers(ctx, arguments),
        _ => ToolCallResult::error(format!("Unknown tool: {}", tool_name)),
    }
}

// â”€â”€â”€ search_reindex_definitions handler â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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
    if let Err(e) = crate::definitions::save_definition_index(&new_index) {
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

// â”€â”€â”€ search_grep handler â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn handle_search_grep(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
    let terms_str = match args.get("terms").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return ToolCallResult::error("Missing required parameter: terms".to_string()),
    };

    // Check dir parameter â€” must match server dir or be absent
    if let Some(dir) = args.get("dir").and_then(|v| v.as_str()) {
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
    }

    let ext_filter = args.get("ext").and_then(|v| v.as_str()).map(|s| s.to_string());
    let mode_and = args.get("mode").and_then(|v| v.as_str()) == Some("and");
    let use_regex = args.get("regex").and_then(|v| v.as_bool()).unwrap_or(false);
    let use_phrase = args.get("phrase").and_then(|v| v.as_bool()).unwrap_or(false);
    let use_substring = args.get("substring").and_then(|v| v.as_bool()).unwrap_or(false);
    let show_lines = args.get("showLines").and_then(|v| v.as_bool()).unwrap_or(false);
    let context_lines = args.get("contextLines").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let max_results = args.get("maxResults").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
    let count_only = args.get("countOnly").and_then(|v| v.as_bool()).unwrap_or(false);
    let exclude_dir: Vec<String> = args.get("excludeDir")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let exclude: Vec<String> = args.get("exclude")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    let search_start = Instant::now();

    // â”€â”€â”€ Mutual exclusivity check â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    if use_substring && (use_regex || use_phrase) {
        return ToolCallResult::error(
            "substring is mutually exclusive with regex and phrase".to_string(),
        );
    }

    // â”€â”€â”€ Substring: check if trigram index needs rebuild â”€â”€â”€â”€â”€
    if use_substring {
        let needs_rebuild = ctx.index.read().map(|idx| idx.trigram_dirty).unwrap_or(false);
        if needs_rebuild {
            if let Ok(mut idx) = ctx.index.write() {
                if idx.trigram_dirty {
                    idx.trigram = build_trigram_index(&idx.index);
                    idx.trigram_dirty = false;
                    eprintln!("[substring] Rebuilt trigram index: {} tokens, {} trigrams",
                        idx.trigram.tokens.len(), idx.trigram.trigram_map.len());
                }
            }
        }
    }

    let index = match ctx.index.read() {
        Ok(idx) => idx,
        Err(e) => return ToolCallResult::error(format!("Failed to acquire index lock: {}", e)),
    };

    // â”€â”€â”€ Substring search mode â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    if use_substring {
        return handle_substring_search(ctx, &index, &terms_str, &ext_filter, &exclude_dir, &exclude,
            show_lines, context_lines, max_results, mode_and, count_only, search_start);
    }

    // â”€â”€â”€ Phrase search mode â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    if use_phrase {
        return handle_phrase_search(
            &index, &terms_str, &ext_filter, &exclude_dir, &exclude,
            show_lines, context_lines, max_results, count_only, search_start,
        );
    }

    // â”€â”€â”€ Normal token search â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let raw_terms: Vec<String> = terms_str
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    // If regex mode, expand each pattern
    let terms: Vec<String> = if use_regex {
        let mut expanded = Vec::new();
        for pat in &raw_terms {
            match regex::Regex::new(&format!("(?i)^{}$", pat)) {
                Ok(re) => {
                    let matching: Vec<String> = index.index.keys()
                        .filter(|k| re.is_match(k))
                        .cloned()
                        .collect();
                    expanded.extend(matching);
                }
                Err(e) => return ToolCallResult::error(format!("Invalid regex '{}': {}", pat, e)),
            }
        }
        expanded
    } else {
        raw_terms.clone()
    };

    let total_docs = index.files.len() as f64;
    let search_mode = if use_regex { "regex" } else if mode_and { "and" } else { "or" };
    let term_count_for_all = if use_regex { raw_terms.len() } else { terms.len() };

    // Collect per-file scores
    let mut file_scores: HashMap<u32, FileScoreEntry> = HashMap::new();

    for term in &terms {
        if let Some(postings) = index.index.get(term.as_str()) {
            let doc_freq = postings.len() as f64;
            let idf = (total_docs / doc_freq).ln();

            for posting in postings {
                let file_path = &index.files[posting.file_id as usize];

                // Extension filter
                if let Some(ref ext) = ext_filter {
                    let matches = Path::new(file_path)
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case(ext));
                    if !matches { continue; }
                }

                // Exclude dir filter
                if exclude_dir.iter().any(|excl| {
                    file_path.to_lowercase().contains(&excl.to_lowercase())
                }) { continue; }

                // Exclude pattern filter
                if exclude.iter().any(|excl| {
                    file_path.to_lowercase().contains(&excl.to_lowercase())
                }) { continue; }

                let occurrences = posting.lines.len();
                let file_total = if (posting.file_id as usize) < index.file_token_counts.len() {
                    index.file_token_counts[posting.file_id as usize] as f64
                } else {
                    1.0
                };
                let tf = occurrences as f64 / file_total;
                let tf_idf = tf * idf;

                let entry = file_scores.entry(posting.file_id).or_insert(FileScoreEntry {
                    file_path: file_path.clone(),
                    lines: Vec::new(),
                    tf_idf: 0.0,
                    occurrences: 0,
                    terms_matched: 0,
                });
                entry.tf_idf += tf_idf;
                entry.occurrences += occurrences;
                entry.lines.extend_from_slice(&posting.lines);
                entry.terms_matched += 1;
            }
        }
    }

    // Filter by AND mode
    let mut results: Vec<FileScoreEntry> = file_scores
        .into_values()
        .filter(|fs| !mode_and || fs.terms_matched >= term_count_for_all)
        .collect();

    // Sort/dedup lines
    for result in &mut results {
        result.lines.sort();
        result.lines.dedup();
    }

    // Sort by TF-IDF descending
    results.sort_by(|a, b| b.tf_idf.partial_cmp(&a.tf_idf).unwrap_or(std::cmp::Ordering::Equal));

    let total_files = results.len();
    let total_occurrences: usize = results.iter().map(|r| r.occurrences).sum();

    // Apply max_results
    if max_results > 0 {
        results.truncate(max_results);
    }

    let search_elapsed = search_start.elapsed();

    if count_only {
        let output = json!({
            "summary": {
                "totalFiles": total_files,
                "totalOccurrences": total_occurrences,
                "termsSearched": terms,
                "searchMode": search_mode,
                "indexFiles": index.files.len(),
                "indexTokens": index.index.len(),
                "searchTimeMs": search_elapsed.as_secs_f64() * 1000.0,
                "indexLoadTimeMs": 0.0
            }
        });
        return ToolCallResult::success(serde_json::to_string(&output).unwrap());
    }

    // Build JSON output
    let files_json: Vec<Value> = results.iter().map(|r| {
        let mut file_obj = json!({
            "path": r.file_path,
            "score": (r.tf_idf * 10000.0).round() / 10000.0,
            "occurrences": r.occurrences,
            "termsMatched": format!("{}/{}", r.terms_matched, terms.len()),
            "lines": r.lines,
        });

        if show_lines
            && let Ok(content) = std::fs::read_to_string(&r.file_path) {
                file_obj["lineContent"] = build_line_content_from_matches(&content, &r.lines, context_lines);
            }

        file_obj
    }).collect();

    let output = json!({
        "files": files_json,
        "summary": {
            "totalFiles": total_files,
            "totalOccurrences": total_occurrences,
            "termsSearched": terms,
            "searchMode": search_mode,
            "indexFiles": index.files.len(),
            "indexTokens": index.index.len(),
            "searchTimeMs": search_elapsed.as_secs_f64() * 1000.0,
            "indexLoadTimeMs": 0.0
        }
    });

    ToolCallResult::success(serde_json::to_string(&output).unwrap())
}

struct FileScoreEntry {
    file_path: String,
    lines: Vec<u32>,
    tf_idf: f64,
    occurrences: usize,
    terms_matched: usize,
}
/// Merge-intersect two sorted u32 slices. Returns sorted intersection.
fn sorted_intersect(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut result = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Equal => { result.push(a[i]); i += 1; j += 1; }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    result
}

/// Substring search using the trigram index.
fn handle_substring_search(
    _ctx: &HandlerContext,
    index: &ContentIndex,
    terms_str: &str,
    ext_filter: &Option<String>,
    exclude_dir: &[String],
    exclude: &[String],
    show_lines: bool,
    context_lines: usize,
    max_results_param: usize,
    mode_and: bool,
    count_only: bool,
    _search_start: Instant,
) -> ToolCallResult {
    let max_results = if max_results_param == 0 { 0 } else { max_results_param };

    let raw_terms: Vec<String> = terms_str
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    if raw_terms.is_empty() {
        return ToolCallResult::error("No search terms provided".to_string());
    }

    let trigram_idx = &index.trigram;
    let total_docs = index.files.len() as f64;
    let search_mode = if mode_and { "and" } else { "or" };

    // Track warnings
    let mut warnings: Vec<String> = Vec::new();
    let has_short_query = raw_terms.iter().any(|t| t.len() < 4);
    if has_short_query {
        warnings.push("Short substring query (<4 chars) may return broad results".to_string());
    }

    // For each term, find matching tokens via trigram index
    let mut all_matched_tokens: Vec<String> = Vec::new();
    let mut file_scores: HashMap<u32, FileScoreEntry> = HashMap::new();
    let term_count = raw_terms.len();

    for term in &raw_terms {
        // Find tokens that contain this term as a substring
        let matched_token_indices: Vec<u32> = if term.len() < 3 {
            // Linear scan for very short terms (no trigrams possible)
            trigram_idx.tokens.iter().enumerate()
                .filter(|(_, tok)| tok.contains(term.as_str()))
                .map(|(i, _)| i as u32)
                .collect()
        } else {
            // Use trigram index: intersect posting lists for all trigrams of the term
            let trigrams = generate_trigrams(term);
            if trigrams.is_empty() {
                Vec::new()
            } else {
                // Get candidate token indices by intersecting trigram posting lists
                let mut candidates: Option<Vec<u32>> = None;
                for tri in &trigrams {
                    if let Some(posting_list) = trigram_idx.trigram_map.get(tri) {
                        candidates = Some(match candidates {
                            None => posting_list.clone(),
                            Some(prev) => sorted_intersect(&prev, posting_list),
                        });
                    } else {
                        // Trigram not found â†’ no candidates
                        candidates = Some(Vec::new());
                        break;
                    }
                }

                let candidate_indices = candidates.unwrap_or_default();

                // Verify candidates: check that the token actually contains the substring
                candidate_indices.into_iter()
                    .filter(|&idx| {
                        if let Some(tok) = trigram_idx.tokens.get(idx as usize) {
                            tok.contains(term.as_str())
                        } else {
                            false
                        }
                    })
                    .collect()
            }
        };

        // Collect matched token names
        let matched_tokens: Vec<String> = matched_token_indices.iter()
            .filter_map(|&idx| trigram_idx.tokens.get(idx as usize).cloned())
            .collect();
        all_matched_tokens.extend(matched_tokens.iter().cloned());

        // For each matched token, look up in main inverted index to get file postings
        for token in &matched_tokens {
            let token_key: &str = token.as_str();
            if let Some(postings) = index.index.get(token_key) {
                let doc_freq = postings.len() as f64;
                let idf = if doc_freq > 0.0 { (total_docs / doc_freq).ln() } else { 0.0 };

                for posting in postings {
                    let file_path = &index.files[posting.file_id as usize];

                    // Extension filter
                    if let Some(ext) = ext_filter {
                        let matches = Path::new(file_path)
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case(ext));
                        if !matches { continue; }
                    }

                    // Exclude dir filter
                    if exclude_dir.iter().any(|excl| {
                        file_path.to_lowercase().contains(&excl.to_lowercase())
                    }) { continue; }

                    // Exclude pattern filter
                    if exclude.iter().any(|excl| {
                        file_path.to_lowercase().contains(&excl.to_lowercase())
                    }) { continue; }

                    let occurrences = posting.lines.len();
                    let file_total = if (posting.file_id as usize) < index.file_token_counts.len() {
                        index.file_token_counts[posting.file_id as usize] as f64
                    } else {
                        1.0
                    };
                    let tf = occurrences as f64 / file_total;
                    let tf_idf = tf * idf;

                    let entry = file_scores.entry(posting.file_id).or_insert(FileScoreEntry {
                        file_path: file_path.clone(),
                        lines: Vec::new(),
                        tf_idf: 0.0,
                        occurrences: 0,
                        terms_matched: 0,
                    });
                    entry.tf_idf += tf_idf;
                    entry.occurrences += occurrences;
                    entry.lines.extend_from_slice(&posting.lines);
                    entry.terms_matched += 1;
                }
            }
        }
    }

    // Dedup matched tokens
    all_matched_tokens.sort();
    all_matched_tokens.dedup();

    // Filter by AND mode
    let mut results: Vec<FileScoreEntry> = file_scores
        .into_values()
        .filter(|fs| !mode_and || fs.terms_matched >= term_count)
        .collect();

    // Sort/dedup lines
    for result in &mut results {
        result.lines.sort();
        result.lines.dedup();
    }

    // Sort by TF-IDF descending
    results.sort_by(|a, b| b.tf_idf.partial_cmp(&a.tf_idf).unwrap_or(std::cmp::Ordering::Equal));

    let total_files = results.len();
    let total_occurrences: usize = results.iter().map(|r| r.occurrences).sum();

    // Apply max_results
    if max_results > 0 {
        results.truncate(max_results);
    }

    if count_only {
        let mut summary = json!({
            "totalFiles": total_files,
            "totalOccurrences": total_occurrences,
            "termsSearched": raw_terms,
            "searchMode": format!("substring-{}", search_mode),
            "matchedTokens": all_matched_tokens,
        });
        if !warnings.is_empty() {
            summary["warning"] = json!(warnings[0]);
        }
        let output = json!({
            "summary": summary
        });
        return ToolCallResult::success(output.to_string());
    }

    // Build JSON output
    let files_json: Vec<Value> = results.iter().map(|r| {
        let mut file_obj = json!({
            "path": r.file_path,
            "score": (r.tf_idf * 10000.0).round() / 10000.0,
            "occurrences": r.occurrences,
            "lines": r.lines,
        });

        if show_lines {
            if let Ok(content) = std::fs::read_to_string(&r.file_path) {
                file_obj["lineContent"] = build_line_content_from_matches(&content, &r.lines, context_lines);
            }
        }

        file_obj
    }).collect();

    let mut summary = json!({
        "totalFiles": total_files,
        "totalOccurrences": total_occurrences,
        "termsSearched": raw_terms,
        "searchMode": format!("substring-{}", search_mode),
        "matchedTokens": all_matched_tokens,
    });
    if !warnings.is_empty() {
        summary["warning"] = json!(warnings[0]);
    }
    let output = json!({
        "files": files_json,
        "summary": summary
    });

    ToolCallResult::success(output.to_string())
}


fn handle_phrase_search(
    index: &ContentIndex,
    phrase: &str,
    ext_filter: &Option<String>,
    exclude_dir: &[String],
    exclude: &[String],
    show_lines: bool,
    context_lines: usize,
    max_results: usize,
    count_only: bool,
    search_start: Instant,
) -> ToolCallResult {
    let phrase_lower = phrase.to_lowercase();
    let phrase_tokens = tokenize(&phrase_lower, 2);

    if phrase_tokens.is_empty() {
        return ToolCallResult::error(format!(
            "Phrase '{}' has no indexable tokens (min length 2)", phrase
        ));
    }

    let phrase_regex_pattern = phrase_tokens.iter()
        .map(|t| regex::escape(t))
        .collect::<Vec<_>>()
        .join(r"\s+");
    let phrase_re = match regex::Regex::new(&format!("(?i){}", phrase_regex_pattern)) {
        Ok(r) => r,
        Err(e) => return ToolCallResult::error(format!("Failed to build phrase regex: {}", e)),
    };

    // Step 1: Find candidate files via AND search
    let mut candidate_file_ids: Option<std::collections::HashSet<u32>> = None;
    for token in &phrase_tokens {
        if let Some(postings) = index.index.get(token.as_str()) {
            let file_ids: std::collections::HashSet<u32> = postings.iter()
                .filter(|p| {
                    let path = &index.files[p.file_id as usize];
                    if let Some(ext) = ext_filter {
                        let m = Path::new(path).extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case(ext));
                        if !m { return false; }
                    }
                    if exclude_dir.iter().any(|excl| path.to_lowercase().contains(&excl.to_lowercase())) {
                        return false;
                    }
                    if exclude.iter().any(|excl| path.to_lowercase().contains(&excl.to_lowercase())) {
                        return false;
                    }
                    true
                })
                .map(|p| p.file_id)
                .collect();
            candidate_file_ids = Some(match candidate_file_ids {
                Some(existing) => existing.intersection(&file_ids).cloned().collect(),
                None => file_ids,
            });
        } else {
            candidate_file_ids = Some(std::collections::HashSet::new());
            break;
        }
    }

    let candidates = candidate_file_ids.unwrap_or_default();

    // Step 2: Verify phrase (read file once, cache content for show_lines)
    struct PhraseMatch {
        file_path: String,
        lines: Vec<u32>,
        content: Option<String>, // cached for show_lines to avoid re-reading
    }
    let mut results: Vec<PhraseMatch> = Vec::new();

    for &file_id in &candidates {
        let file_path = &index.files[file_id as usize];
        if let Ok(content) = std::fs::read_to_string(file_path)
            && phrase_re.is_match(&content) {
                let mut matching_lines = Vec::new();
                for (line_num, line) in content.lines().enumerate() {
                    if phrase_re.is_match(line) {
                        matching_lines.push((line_num + 1) as u32);
                    }
                }
                if !matching_lines.is_empty() {
                    results.push(PhraseMatch {
                        file_path: file_path.clone(),
                        lines: matching_lines,
                        // Only keep content in memory if we'll need it for show_lines
                        content: if show_lines { Some(content) } else { None },
                    });
                }
            }
    }

    let total_files = results.len();
    let total_occurrences: usize = results.iter().map(|r| r.lines.len()).sum();

    if max_results > 0 {
        results.truncate(max_results);
    }

    let search_elapsed = search_start.elapsed();

    if count_only {
        let output = json!({
            "summary": {
                "totalFiles": total_files,
                "totalOccurrences": total_occurrences,
                "termsSearched": [phrase],
                "searchMode": "phrase",
                "indexFiles": index.files.len(),
                "indexTokens": index.index.len(),
                "searchTimeMs": search_elapsed.as_secs_f64() * 1000.0,
                "indexLoadTimeMs": 0.0
            }
        });
        return ToolCallResult::success(serde_json::to_string(&output).unwrap());
    }

    let files_json: Vec<Value> = results.iter().map(|r| {
        let mut file_obj = json!({
            "path": r.file_path,
            "occurrences": r.lines.len(),
            "lines": r.lines,
        });

        if show_lines {
            // Use cached content from phrase verification (no second read)
            if let Some(ref content) = r.content {
                file_obj["lineContent"] = build_line_content_from_matches(content, &r.lines, context_lines);
            }
        }

        file_obj
    }).collect();

    let output = json!({
        "files": files_json,
        "summary": {
            "totalFiles": total_files,
            "totalOccurrences": total_occurrences,
            "termsSearched": [phrase],
            "searchMode": "phrase",
            "indexFiles": index.files.len(),
            "indexTokens": index.index.len(),
            "searchTimeMs": search_elapsed.as_secs_f64() * 1000.0,
            "indexLoadTimeMs": 0.0
        }
    });

    ToolCallResult::success(serde_json::to_string(&output).unwrap())
}

// â”€â”€â”€ search_find handler â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn handle_search_find(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
    let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return ToolCallResult::error("Missing required parameter: pattern".to_string()),
    };

    let dir = args.get("dir").and_then(|v| v.as_str()).unwrap_or(&ctx.server_dir).to_string();
    let ext = args.get("ext").and_then(|v| v.as_str()).map(|s| s.to_string());
    let contents = args.get("contents").and_then(|v| v.as_bool()).unwrap_or(false);
    let use_regex = args.get("regex").and_then(|v| v.as_bool()).unwrap_or(false);
    let ignore_case = args.get("ignoreCase").and_then(|v| v.as_bool()).unwrap_or(false);
    let max_depth = args.get("maxDepth").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let count_only = args.get("countOnly").and_then(|v| v.as_bool()).unwrap_or(false);

    let start = Instant::now();

    let search_pattern = if ignore_case {
        pattern.to_lowercase()
    } else {
        pattern.clone()
    };

    let re = if use_regex {
        match regex::Regex::new(&if ignore_case {
            format!("(?i){}", &pattern)
        } else {
            pattern.clone()
        }) {
            Ok(r) => Some(r),
            Err(e) => return ToolCallResult::error(format!("Invalid regex: {}", e)),
        }
    } else {
        None
    };

    let root = Path::new(&dir);
    if !root.exists() {
        return ToolCallResult::error(format!("Directory does not exist: {}", dir));
    }

    let mut results: Vec<Value> = Vec::new();
    let mut match_count = 0usize;
    let mut file_count = 0usize;

    let mut builder = ignore::WalkBuilder::new(root);
    builder.hidden(false);
    if max_depth > 0 {
        builder.max_depth(Some(max_depth));
    }

    if contents {
        for entry in builder.build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_some_and(|ft| ft.is_file()) { continue; }
            if let Some(ref ext_f) = ext {
                let matches_ext = entry.path().extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case(ext_f));
                if !matches_ext { continue; }
            }
            file_count += 1;
            let content = match std::fs::read_to_string(entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let matched = if let Some(ref re) = re {
                re.is_match(&content)
            } else if ignore_case {
                content.to_lowercase().contains(&search_pattern)
            } else {
                content.contains(&search_pattern)
            };
            if matched {
                match_count += 1;
                if !count_only {
                    let mut lines = Vec::new();
                    for (line_num, line) in content.lines().enumerate() {
                        let line_matched = if let Some(ref re) = re {
                            re.is_match(line)
                        } else if ignore_case {
                            line.to_lowercase().contains(&search_pattern)
                        } else {
                            line.contains(&search_pattern)
                        };
                        if line_matched {
                            lines.push(json!({
                                "line": line_num + 1,
                                "text": line.trim(),
                            }));
                        }
                    }
                    results.push(json!({
                        "path": entry.path().display().to_string(),
                        "matches": lines,
                    }));
                }
            }
        }
    } else {
        for entry in builder.build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            file_count += 1;
            let name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            if let Some(ref ext_f) = ext {
                let matches_ext = entry.path().extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case(ext_f));
                if !matches_ext { continue; }
            }
            let search_name = if ignore_case { name.to_lowercase() } else { name.clone() };
            let matched = if let Some(ref re) = re {
                re.is_match(&search_name)
            } else {
                search_name.contains(&search_pattern)
            };
            if matched {
                match_count += 1;
                if !count_only {
                    results.push(json!({
                        "path": entry.path().display().to_string(),
                    }));
                }
            }
        }
    }

    let elapsed = start.elapsed();

    let output = json!({
        "files": results,
        "summary": {
            "totalMatches": match_count,
            "totalFilesScanned": file_count,
            "searchTimeMs": elapsed.as_secs_f64() * 1000.0,
        }
    });

    ToolCallResult::success(serde_json::to_string(&output).unwrap())
}

// â”€â”€â”€ search_fast handler â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn handle_search_fast(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
    let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return ToolCallResult::error("Missing required parameter: pattern".to_string()),
    };

    let dir = args.get("dir").and_then(|v| v.as_str()).unwrap_or(&ctx.server_dir).to_string();
    let ext = args.get("ext").and_then(|v| v.as_str()).map(|s| s.to_string());
    let use_regex = args.get("regex").and_then(|v| v.as_bool()).unwrap_or(false);
    let ignore_case = args.get("ignoreCase").and_then(|v| v.as_bool()).unwrap_or(false);
    let dirs_only = args.get("dirsOnly").and_then(|v| v.as_bool()).unwrap_or(false);
    let files_only = args.get("filesOnly").and_then(|v| v.as_bool()).unwrap_or(false);
    let count_only = args.get("countOnly").and_then(|v| v.as_bool()).unwrap_or(false);

    let start = Instant::now();

    // Load file index
    let index = match crate::load_index(&dir) {
        Some(idx) => idx,
        None => {
            // Auto-build
            info!(dir = %dir, "No file index found, building automatically");
            let new_index = crate::build_index(&crate::IndexArgs {
                dir: dir.clone(),
                max_age_hours: 24,
                hidden: false,
                no_ignore: false,
                threads: 0,
            });
            let _ = crate::save_index(&new_index);
            new_index
        }
    };

    let search_pattern = if ignore_case { pattern.to_lowercase() } else { pattern.clone() };
    let re = if use_regex {
        match regex::Regex::new(&if ignore_case {
            format!("(?i){}", &pattern)
        } else {
            pattern.clone()
        }) {
            Ok(r) => Some(r),
            Err(e) => return ToolCallResult::error(format!("Invalid regex: {}", e)),
        }
    } else {
        None
    };

    let mut results: Vec<Value> = Vec::new();
    let mut match_count = 0usize;

    for entry in &index.entries {
        if dirs_only && !entry.is_dir { continue; }
        if files_only && entry.is_dir { continue; }

        if let Some(ref ext_f) = ext {
            let path = Path::new(&entry.path);
            let matches_ext = path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(ext_f));
            if !matches_ext { continue; }
        }

        let name = Path::new(&entry.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let search_name = if ignore_case { name.to_lowercase() } else { name.to_string() };

        let matched = if let Some(ref re) = re {
            re.is_match(&search_name)
        } else {
            search_name.contains(&search_pattern)
        };

        if matched {
            match_count += 1;
            if !count_only {
                results.push(json!({
                    "path": entry.path,
                    "size": entry.size,
                    "isDir": entry.is_dir,
                }));
            }
        }
    }

    let elapsed = start.elapsed();

    let output = json!({
        "files": results,
        "summary": {
            "totalMatches": match_count,
            "totalIndexed": index.entries.len(),
            "searchTimeMs": elapsed.as_secs_f64() * 1000.0,
        }
    });

    ToolCallResult::success(serde_json::to_string(&output).unwrap())
}

// â”€â”€â”€ search_info handler â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn handle_search_info() -> ToolCallResult {
    let info = cmd_info_json();
    ToolCallResult::success(serde_json::to_string(&info).unwrap())
}

// â”€â”€â”€ search_reindex handler â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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
    if let Err(e) = save_content_index(&new_index) {
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

// â”€â”€â”€ helper: inject body source code into definition JSON object â”€â”€â”€â”€â”€

/// Build compact grouped lineContent for search_grep from raw file content.
/// Computes context windows around match lines, then groups consecutive lines
/// into `[{startLine, lines[], matchIndices[]}]`.
fn build_line_content_from_matches(
    content: &str,
    match_lines: &[u32],
    context_lines: usize,
) -> Value {
    let lines_vec: Vec<&str> = content.lines().collect();
    let total_lines = lines_vec.len();

    let mut lines_to_show = std::collections::BTreeSet::new();
    let mut match_lines_set = std::collections::HashSet::new();

    for &ln in match_lines {
        let idx = (ln as usize).saturating_sub(1);
        if idx < total_lines {
            match_lines_set.insert(idx);
            let s = idx.saturating_sub(context_lines);
            let e = (idx + context_lines).min(total_lines - 1);
            for i in s..=e { lines_to_show.insert(i); }
        }
    }

    build_grouped_line_content(&lines_to_show, &lines_vec, &match_lines_set)
}

/// Groups consecutive lines into compact chunks: `[{startLine, lines[], matchIndices[]}]`.
fn build_grouped_line_content(
    lines_to_show: &std::collections::BTreeSet<usize>,
    lines_vec: &[&str],
    match_lines_set: &std::collections::HashSet<usize>,
) -> Value {
    let mut groups: Vec<Value> = Vec::new();
    let mut current_group_start: Option<usize> = None;
    let mut current_group_lines: Vec<&str> = Vec::new();
    let mut current_group_matches: Vec<usize> = Vec::new();

    let ordered_lines: Vec<usize> = lines_to_show.iter().cloned().collect();

    for (i, &idx) in ordered_lines.iter().enumerate() {
        let is_consecutive = i > 0 && idx == ordered_lines[i - 1] + 1;

        if !is_consecutive && !current_group_lines.is_empty() {
            let mut group = json!({
                "startLine": current_group_start.unwrap() + 1,
                "lines": current_group_lines,
            });
            if !current_group_matches.is_empty() {
                group["matchIndices"] = json!(current_group_matches);
            }
            groups.push(group);
            current_group_lines = Vec::new();
            current_group_matches = Vec::new();
        }

        if current_group_lines.is_empty() {
            current_group_start = Some(idx);
        }

        if match_lines_set.contains(&idx) {
            current_group_matches.push(current_group_lines.len());
        }
        current_group_lines.push(lines_vec[idx]);
    }

    if !current_group_lines.is_empty() {
        let mut group = json!({
            "startLine": current_group_start.unwrap() + 1,
            "lines": current_group_lines,
        });
        if !current_group_matches.is_empty() {
            group["matchIndices"] = json!(current_group_matches);
        }
        groups.push(group);
    }

    json!(groups)
}

fn inject_body_into_obj(
    obj: &mut Value,
    file_path: &str,
    line_start: u32,
    line_end: u32,
    file_cache: &mut HashMap<String, Option<String>>,
    total_body_lines_emitted: &mut usize,
    max_body_lines: usize,
    max_total_body_lines: usize,
) {
    // Check total budget
    if max_total_body_lines > 0 && *total_body_lines_emitted >= max_total_body_lines {
        obj["bodyOmitted"] = json!("total body lines budget exceeded");
        return;
    }

    // Read file via cache
    let content_opt = file_cache
        .entry(file_path.to_string())
        .or_insert_with(|| std::fs::read_to_string(file_path).ok())
        .clone();

    match content_opt {
        None => {
            obj["bodyError"] = json!("failed to read file");
        }
        Some(content) => {
            let lines_vec: Vec<&str> = content.lines().collect();
            let total_file_lines = lines_vec.len();

            // 1-based to 0-based
            let start_idx = (line_start as usize).saturating_sub(1);
            let end_idx = (line_end as usize).min(total_file_lines);

            // Stale data check
            if line_end as usize > total_file_lines {
                obj["bodyWarning"] = json!(format!(
                    "definition claims line_end={} but file has only {} lines (stale index?)",
                    line_end, total_file_lines
                ));
            }

            let body_lines: Vec<&str> = if start_idx < total_file_lines {
                lines_vec[start_idx..end_idx].to_vec()
            } else {
                vec![]
            };

            let total_body_lines_in_def = body_lines.len();

            // Calculate remaining budget
            let remaining_budget = if max_total_body_lines == 0 {
                usize::MAX
            } else {
                max_total_body_lines.saturating_sub(*total_body_lines_emitted)
            };

            // Effective max per definition
            let effective_max = if max_body_lines == 0 {
                remaining_budget
            } else {
                max_body_lines.min(remaining_budget)
            };

            let truncated = total_body_lines_in_def > effective_max;
            let lines_to_emit = if truncated { effective_max } else { total_body_lines_in_def };

            let body_array: Vec<&str> = body_lines[..lines_to_emit].to_vec();

            obj["bodyStartLine"] = json!(start_idx + 1);
            obj["body"] = json!(body_array);

            if truncated {
                obj["bodyTruncated"] = json!(true);
                obj["totalBodyLines"] = json!(total_body_lines_in_def);
            }

            *total_body_lines_emitted += lines_to_emit;
        }
    }
}

// â”€â”€â”€ search_definitions handler â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn handle_search_definitions(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
    let def_index = match &ctx.def_index {
        Some(idx) => idx,
        None => return ToolCallResult::error(
            "Definition index not available. Start server with --definitions flag.".to_string()
        ),
    };

    let index = match def_index.read() {
        Ok(idx) => idx,
        Err(e) => return ToolCallResult::error(format!("Failed to acquire definition index lock: {}", e)),
    };

    let search_start = Instant::now();

    let name_filter = args.get("name").and_then(|v| v.as_str());
    let kind_filter = args.get("kind").and_then(|v| v.as_str());
    let attribute_filter = args.get("attribute").and_then(|v| v.as_str());
    let base_type_filter = args.get("baseType").and_then(|v| v.as_str());
    let file_filter = args.get("file").and_then(|v| v.as_str());
    let parent_filter = args.get("parent").and_then(|v| v.as_str());
    let contains_line = args.get("containsLine").and_then(|v| v.as_u64()).map(|v| v as u32);
    let use_regex = args.get("regex").and_then(|v| v.as_bool()).unwrap_or(false);
    let max_results = args.get("maxResults").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
    let exclude_dir: Vec<String> = args.get("excludeDir")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let include_body = args.get("includeBody").and_then(|v| v.as_bool()).unwrap_or(false);
    let max_body_lines = args.get("maxBodyLines").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
    let max_total_body_lines = args.get("maxTotalBodyLines").and_then(|v| v.as_u64()).unwrap_or(500) as usize;

    // â”€â”€â”€ containsLine: find containing method/class by line number â”€â”€â”€
    if let Some(line_num) = contains_line {
        if file_filter.is_none() {
            return ToolCallResult::error(
                "containsLine requires 'file' parameter to identify the file.".to_string()
            );
        }
        let file_substr = file_filter.unwrap().to_lowercase();

        // Find matching file(s)
        let mut containing_defs: Vec<Value> = Vec::new();
        let mut file_cache: HashMap<String, Option<String>> = HashMap::new();
        let mut total_body_lines_emitted: usize = 0;
        for (file_id, file_path) in index.files.iter().enumerate() {
            if !file_path.to_lowercase().contains(&file_substr) {
                continue;
            }
            // Get all definitions in this file
            if let Some(def_indices) = index.file_index.get(&(file_id as u32)) {
                // Find all definitions that contain this line, sorted by specificity
                // (innermost first = smallest line range)
                let mut matching: Vec<&DefinitionEntry> = def_indices.iter()
                    .filter_map(|&di| index.definitions.get(di as usize))
                    .filter(|d| d.line_start <= line_num && d.line_end >= line_num)
                    .collect();

                // Sort by range size (smallest first = most specific)
                matching.sort_by_key(|d| d.line_end - d.line_start);

                for def in &matching {
                    let mut obj = json!({
                        "name": def.name,
                        "kind": def.kind.as_str(),
                        "file": file_path,
                        "lines": format!("{}-{}", def.line_start, def.line_end),
                    });
                    if let Some(ref parent) = def.parent {
                        obj["parent"] = json!(parent);
                    }
                    if let Some(ref sig) = def.signature {
                        obj["signature"] = json!(sig);
                    }
                    if !def.modifiers.is_empty() {
                        obj["modifiers"] = json!(def.modifiers);
                    }
                    if include_body {
                        inject_body_into_obj(
                            &mut obj, file_path, def.line_start, def.line_end,
                            &mut file_cache, &mut total_body_lines_emitted,
                            max_body_lines, max_total_body_lines,
                        );
                    }
                    containing_defs.push(obj);
                }
            }
        }

        let search_elapsed = search_start.elapsed();
        let mut summary = json!({
            "totalResults": containing_defs.len(),
            "searchTimeMs": search_elapsed.as_secs_f64() * 1000.0,
        });
        if include_body {
            summary["totalBodyLinesReturned"] = json!(total_body_lines_emitted);
        }
        let output = json!({
            "containingDefinitions": containing_defs,
            "query": {
                "file": file_filter.unwrap(),
                "line": line_num,
            },
            "summary": summary,
        });
        return ToolCallResult::success(serde_json::to_string(&output).unwrap());
    }

    // Start with candidate indices
    let mut candidate_indices: Option<Vec<u32>> = None;

    // Filter by kind first (most selective usually)
    if let Some(kind_str) = kind_filter {
        match kind_str.parse::<DefinitionKind>() {
            Ok(kind) => {
                if let Some(indices) = index.kind_index.get(&kind) {
                    candidate_indices = Some(indices.clone());
                } else {
                    candidate_indices = Some(Vec::new());
                }
            }
            Err(e) => {
                return ToolCallResult::error(e);
            }
        }
    }

    // Filter by attribute
    if let Some(attr) = attribute_filter {
        let attr_lower = attr.to_lowercase();
        if let Some(indices) = index.attribute_index.get(&attr_lower) {
            candidate_indices = Some(match candidate_indices {
                Some(existing) => {
                    let set: std::collections::HashSet<u32> = indices.iter().cloned().collect();
                    existing.into_iter().filter(|i| set.contains(i)).collect()
                }
                None => indices.clone(),
            });
        } else {
            candidate_indices = Some(Vec::new());
        }
    }

    // Filter by base type
    if let Some(bt) = base_type_filter {
        let bt_lower = bt.to_lowercase();
        if let Some(indices) = index.base_type_index.get(&bt_lower) {
            candidate_indices = Some(match candidate_indices {
                Some(existing) => {
                    let set: std::collections::HashSet<u32> = indices.iter().cloned().collect();
                    existing.into_iter().filter(|i| set.contains(i)).collect()
                }
                None => indices.clone(),
            });
        } else {
            candidate_indices = Some(Vec::new());
        }
    }

    // Filter by name
    if let Some(name) = name_filter {
        if use_regex {
            // Regex match against all names in the index
            let re = match regex::Regex::new(&format!("(?i){}", name)) {
                Ok(r) => r,
                Err(e) => return ToolCallResult::error(format!("Invalid regex '{}': {}", name, e)),
            };
            let mut matching_indices = Vec::new();
            for (n, indices) in &index.name_index {
                if re.is_match(n) {
                    matching_indices.extend(indices);
                }
            }
            candidate_indices = Some(match candidate_indices {
                Some(existing) => {
                    let set: std::collections::HashSet<u32> = matching_indices.into_iter().cloned().collect();
                    existing.into_iter().filter(|i| set.contains(i)).collect()
                }
                None => matching_indices.into_iter().cloned().collect(),
            });
        } else {
            // Comma-separated OR search with substring matching
            let terms: Vec<String> = name.split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            let mut matching_indices = Vec::new();
            for (n, indices) in &index.name_index {
                if terms.iter().any(|t| n.contains(t)) {
                    matching_indices.extend(indices);
                }
            }
            candidate_indices = Some(match candidate_indices {
                Some(existing) => {
                    let set: std::collections::HashSet<u32> = matching_indices.into_iter().cloned().collect();
                    existing.into_iter().filter(|i| set.contains(i)).collect()
                }
                None => matching_indices.into_iter().cloned().collect(),
            });
        }
    }

    // If no filters applied, return all definitions (up to max)
    let candidates = candidate_indices.unwrap_or_else(|| {
        (0..index.definitions.len() as u32).collect()
    });

    // Apply remaining filters (file, parent, excludeDir) on actual entries
    let mut results: Vec<&crate::definitions::DefinitionEntry> = candidates.iter()
        .filter_map(|&idx| {
            let def = index.definitions.get(idx as usize)?;
            let file_path = index.files.get(def.file_id as usize)?;

            // File filter
            if let Some(ff) = file_filter
                && !file_path.to_lowercase().contains(&ff.to_lowercase()) {
                    return None;
                }

            // Parent filter
            if let Some(pf) = parent_filter {
                match &def.parent {
                    Some(parent) => {
                        if !parent.to_lowercase().contains(&pf.to_lowercase()) {
                            return None;
                        }
                    }
                    None => return None,
                }
            }

            // Exclude dir
            if exclude_dir.iter().any(|excl| {
                file_path.to_lowercase().contains(&excl.to_lowercase())
            }) {
                return None;
            }

            Some(def)
        })
        .collect();

    let total_results = results.len();

    // Apply max results
    if max_results > 0 && results.len() > max_results {
        results.truncate(max_results);
    }

    let search_elapsed = search_start.elapsed();

    // Build output JSON
    let mut file_cache: HashMap<String, Option<String>> = HashMap::new();
    let mut total_body_lines_emitted: usize = 0;
    let defs_json: Vec<Value> = results.iter().map(|def| {
        let file_path = index.files.get(def.file_id as usize)
            .map(|s| s.as_str())
            .unwrap_or("");

        let mut obj = json!({
            "name": def.name,
            "kind": def.kind.as_str(),
            "file": file_path,
            "lines": format!("{}-{}", def.line_start, def.line_end),
        });

        if !def.modifiers.is_empty() {
            obj["modifiers"] = json!(def.modifiers);
        }
        if !def.attributes.is_empty() {
            obj["attributes"] = json!(def.attributes);
        }
        if !def.base_types.is_empty() {
            obj["baseTypes"] = json!(def.base_types);
        }
        if let Some(ref sig) = def.signature {
            obj["signature"] = json!(sig);
        }
        if let Some(ref parent) = def.parent {
            obj["parent"] = json!(parent);
        }
        if include_body {
            inject_body_into_obj(
                &mut obj, file_path, def.line_start, def.line_end,
                &mut file_cache, &mut total_body_lines_emitted,
                max_body_lines, max_total_body_lines,
            );
        }

        obj
    }).collect();

    let mut summary = json!({
        "totalResults": total_results,
        "returned": defs_json.len(),
        "searchTimeMs": search_elapsed.as_secs_f64() * 1000.0,
        "indexFiles": index.files.len(),
        "totalDefinitions": index.definitions.len(),
    });
    if include_body {
        summary["totalBodyLinesReturned"] = json!(total_body_lines_emitted);
    }
    let output = json!({
        "definitions": defs_json,
        "summary": summary,
    });

    ToolCallResult::success(serde_json::to_string(&output).unwrap())
}

// â”€â”€â”€ search_callers handler â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn handle_search_callers(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
    let def_index = match &ctx.def_index {
        Some(idx) => idx,
        None => return ToolCallResult::error(
            "Definition index not available. Start server with --definitions flag.".to_string()
        ),
    };

    let method_name = match args.get("method").and_then(|v| v.as_str()) {
        Some(m) => m.to_string(),
        None => return ToolCallResult::error("Missing required parameter: method".to_string()),
    };
    let class_filter = args.get("class").and_then(|v| v.as_str()).map(|s| s.to_string());

    let max_depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(3).min(10) as usize;
    let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("up");
    let ext_filter = args.get("ext").and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| ctx.server_ext.clone());
    let resolve_interfaces = args.get("resolveInterfaces").and_then(|v| v.as_bool()).unwrap_or(true);
    let max_callers_per_level = args.get("maxCallersPerLevel").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    let max_total_nodes = args.get("maxTotalNodes").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
    let exclude_dir: Vec<String> = args.get("excludeDir")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let exclude_file: Vec<String> = args.get("excludeFile")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    let search_start = Instant::now();

    let content_index = match ctx.index.read() {
        Ok(idx) => idx,
        Err(e) => return ToolCallResult::error(format!("Failed to acquire content index lock: {}", e)),
    };
    let def_idx = match def_index.read() {
        Ok(idx) => idx,
        Err(e) => return ToolCallResult::error(format!("Failed to acquire definition index lock: {}", e)),
    };

    let limits = CallerLimits { max_callers_per_level, max_total_nodes };
    let node_count = std::sync::atomic::AtomicUsize::new(0);

    // Check for ambiguous method names and generate warning
    let method_lower = method_name.to_lowercase();
    let mut ambiguity_warning: Option<String> = None;
    if class_filter.is_none() {
        if let Some(name_indices) = def_idx.name_index.get(&method_lower) {
            let method_defs: Vec<&DefinitionEntry> = name_indices.iter()
                .filter_map(|&di| def_idx.definitions.get(di as usize))
                .filter(|d| d.kind == DefinitionKind::Method || d.kind == DefinitionKind::Constructor)
                .collect();

            let unique_classes: std::collections::HashSet<&str> = method_defs.iter()
                .filter_map(|d| d.parent.as_deref())
                .collect();

            if unique_classes.len() > 1 {
                let class_list: Vec<&str> = unique_classes.into_iter().collect();
                ambiguity_warning = Some(format!(
                    "Method '{}' found in {} classes: {}. Results may mix callers from different classes. Use 'class' parameter to scope the search.",
                    method_name, class_list.len(), class_list.join(", ")
                ));
            }
        }
    }

    if direction == "up" {
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        let tree = build_caller_tree(
            &method_name,
            class_filter.as_deref(), // pass class scoping from MCP parameter
            max_depth,
            0,
            &content_index,
            &def_idx,
            &ext_filter,
            &exclude_dir,
            &exclude_file,
            resolve_interfaces,
            &mut visited,
            &limits,
            &node_count,
        );

        // Dedup: remove duplicate nodes at root level (can happen with resolveInterfaces)
        let tree = dedup_caller_tree(tree);

        let total_nodes = node_count.load(std::sync::atomic::Ordering::Relaxed);
        let truncated = total_nodes >= max_total_nodes;
        let search_elapsed = search_start.elapsed();
        let mut output = json!({
            "callTree": tree,
            "query": {
                "method": method_name,
                "direction": "up",
                "depth": max_depth,
                "maxCallersPerLevel": max_callers_per_level,
                "maxTotalNodes": max_total_nodes,
            },
            "summary": {
                "nodesVisited": visited.len(),
                "totalNodes": total_nodes,
                "truncated": truncated,
                "searchTimeMs": search_elapsed.as_secs_f64() * 1000.0,
            }
        });
        if let Some(ref warning) = ambiguity_warning {
            output["warning"] = json!(warning);
        }
        if let Some(ref cls) = class_filter {
            output["query"]["class"] = json!(cls);
        }
        ToolCallResult::success(serde_json::to_string(&output).unwrap())
    } else {
        let tree = build_callee_tree(
            &method_name,
            class_filter.as_deref(),
            max_depth,
            0,
            &def_idx,
            &ext_filter,
            &exclude_dir,
            &exclude_file,
            &mut std::collections::HashSet::new(),
            &limits,
            &node_count,
        );

        let total_nodes = node_count.load(std::sync::atomic::Ordering::Relaxed);
        let search_elapsed = search_start.elapsed();
        let mut output = json!({
            "callTree": tree,
            "query": {
                "method": method_name,
                "direction": "down",
                "depth": max_depth,
                "maxCallersPerLevel": max_callers_per_level,
                "maxTotalNodes": max_total_nodes,
            },
            "summary": {
                "totalNodes": total_nodes,
                "searchTimeMs": search_elapsed.as_secs_f64() * 1000.0,
            }
        });
        if let Some(ref warning) = ambiguity_warning {
            output["warning"] = json!(warning);
        }
        if let Some(ref cls) = class_filter {
            output["query"]["class"] = json!(cls);
        }
        ToolCallResult::success(serde_json::to_string(&output).unwrap())
    }
}

/// Remove duplicate nodes from the caller tree (can occur with resolveInterfaces
/// when the same caller is found through multiple interface implementations).
fn dedup_caller_tree(tree: Vec<Value>) -> Vec<Value> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    tree.into_iter()
        .filter(|node| {
            let key = format!(
                "{}.{}.{}",
                node.get("class").and_then(|v| v.as_str()).unwrap_or("?"),
                node.get("method").and_then(|v| v.as_str()).unwrap_or("?"),
                node.get("file").and_then(|v| v.as_str()).unwrap_or("?"),
            );
            seen.insert(key)
        })
        .collect()
}

struct CallerLimits {
    max_callers_per_level: usize,
    max_total_nodes: usize,
}

/// Find the containing method for a given file_id and line number in the definition index.
fn find_containing_method(
    def_idx: &DefinitionIndex,
    file_id: u32,
    line: u32,
) -> Option<(String, Option<String>, u32)> {
    let def_indices = def_idx.file_index.get(&file_id)?;

    let mut best: Option<&DefinitionEntry> = None;
    for &di in def_indices {
        if let Some(def) = def_idx.definitions.get(di as usize) {
            match def.kind {
                DefinitionKind::Method | DefinitionKind::Constructor | DefinitionKind::Property => {}
                _ => continue,
            }
            if def.line_start <= line && def.line_end >= line {
                if let Some(current_best) = best {
                    if (def.line_end - def.line_start) < (current_best.line_end - current_best.line_start) {
                        best = Some(def);
                    }
                } else {
                    best = Some(def);
                }
            }
        }
    }

    best.map(|d| (d.name.clone(), d.parent.clone(), d.line_start))
}

/// Build a caller tree recursively (direction = "up").
/// `parent_class` is used to disambiguate common method names â€” when recursing,
/// we pass the parent class of the method being searched so that we only find
/// callers that actually reference that specific class (not any unrelated class
/// with a method of the same name).
fn build_caller_tree(
    method_name: &str,
    parent_class: Option<&str>,
    max_depth: usize,
    current_depth: usize,
    content_index: &ContentIndex,
    def_idx: &DefinitionIndex,
    ext_filter: &str,
    exclude_dir: &[String],
    exclude_file: &[String],
    resolve_interfaces: bool,
    visited: &mut std::collections::HashSet<String>,
    limits: &CallerLimits,
    node_count: &std::sync::atomic::AtomicUsize,
) -> Vec<Value> {
    if current_depth >= max_depth {
        return Vec::new();
    }
    if node_count.load(std::sync::atomic::Ordering::Relaxed) >= limits.max_total_nodes {
        return Vec::new();
    }

    let method_lower = method_name.to_lowercase();

    // Use class.method as visited key to avoid conflicts between same-named methods
    let visited_key = if let Some(cls) = parent_class {
        format!("{}.{}", cls.to_lowercase(), method_lower)
    } else {
        method_lower.clone()
    };
    if !visited.insert(visited_key) {
        return Vec::new();
    }

    let postings = match content_index.index.get(&method_lower) {
        Some(p) => p,
        None => return Vec::new(),
    };

    // Pre-compute: which content index file_ids contain the parent class token?
    // This filters out files that use the same method name but from a different class.
    // Also check for interface name (IClassName) to handle DI scenarios.
    let parent_file_ids: Option<std::collections::HashSet<u32>> = parent_class.and_then(|cls| {
        let cls_lower = cls.to_lowercase();
        let mut file_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();

        // Add files containing the class name directly
        if let Some(postings) = content_index.index.get(&cls_lower) {
            file_ids.extend(postings.iter().map(|p| p.file_id));
        }

        // Also check for interface name (IClassName pattern for DI)
        let interface_name = format!("i{}", cls_lower);
        if let Some(postings) = content_index.index.get(&interface_name) {
            file_ids.extend(postings.iter().map(|p| p.file_id));
        }

        // Also check if parent implements any interfaces and add files referencing those
        if let Some(name_indices) = def_idx.name_index.get(&cls_lower) {
            for &di in name_indices {
                if let Some(def) = def_idx.definitions.get(di as usize)
                    && (def.kind == DefinitionKind::Class || def.kind == DefinitionKind::Struct) {
                        for bt in &def.base_types {
                            let bt_lower = bt.to_lowercase();
                            if let Some(postings) = content_index.index.get(&bt_lower) {
                                file_ids.extend(postings.iter().map(|p| p.file_id));
                            }
                        }
                    }
            }
        }

        if file_ids.is_empty() { None } else { Some(file_ids) }
    });

    let mut callers: Vec<Value> = Vec::new();
    let mut seen_callers: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut definition_locations: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
    if let Some(name_indices) = def_idx.name_index.get(&method_lower) {
        for &di in name_indices {
            if let Some(def) = def_idx.definitions.get(di as usize)
                && (def.kind == DefinitionKind::Method || def.kind == DefinitionKind::Constructor) {
                    definition_locations.insert((def.file_id, def.line_start));
                }
        }
    }

    for posting in postings {
        if callers.len() >= limits.max_callers_per_level {
            break;
        }
        if node_count.load(std::sync::atomic::Ordering::Relaxed) >= limits.max_total_nodes {
            break;
        }

        // If we have a parent class context, skip files that don't reference that class
        if let Some(ref pids) = parent_file_ids
            && !pids.contains(&posting.file_id) {
                continue;
            }

        let file_path = match content_index.files.get(posting.file_id as usize) {
            Some(p) => p,
            None => continue,
        };

        let matches_ext = Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(ext_filter));
        if !matches_ext { continue; }

        let path_lower = file_path.to_lowercase();
        if exclude_dir.iter().any(|excl| path_lower.contains(&excl.to_lowercase())) { continue; }
        if exclude_file.iter().any(|excl| path_lower.contains(&excl.to_lowercase())) { continue; }

        let def_fid = match def_idx.path_to_id.get(&std::path::PathBuf::from(file_path)).copied() {
            Some(id) => id,
            None => continue,
        };

        for &line in &posting.lines {
            if callers.len() >= limits.max_callers_per_level { break; }
            if node_count.load(std::sync::atomic::Ordering::Relaxed) >= limits.max_total_nodes { break; }

            if definition_locations.contains(&(def_fid, line)) {
                continue;
            }

            if let Some((caller_name, caller_parent, caller_line)) =
                find_containing_method(def_idx, def_fid, line)
            {
                let caller_key = format!("{}.{}",
                    caller_parent.as_deref().unwrap_or("?"),
                    &caller_name
                );

                if seen_callers.contains(&caller_key) {
                    continue;
                }
                seen_callers.insert(caller_key.clone());

                node_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                // Recurse without parent_class filter. The parent_class
                // disambiguation is most useful at the initial level to
                // avoid false positives from common method names. At deeper
                // levels, the visited set prevents infinite loops, and
                // we don't want to miss callers through DI/interfaces.
                let sub_callers = build_caller_tree(
                    &caller_name,
                    None,
                    max_depth,
                    current_depth + 1,
                    content_index,
                    def_idx,
                    ext_filter,
                    exclude_dir,
                    exclude_file,
                    resolve_interfaces,
                    visited,
                    limits,
                    node_count,
                );

                let mut node = json!({
                    "method": caller_name,
                    "line": caller_line,
                    "callSite": line,
                });
                if let Some(ref parent) = caller_parent {
                    node["class"] = json!(parent);
                }
                if let Some(fname) = Path::new(file_path).file_name().and_then(|f| f.to_str()) {
                    node["file"] = json!(fname);
                }
                if !sub_callers.is_empty() {
                    node["callers"] = json!(sub_callers);
                }
                callers.push(node);
            }
        }
    }

    // Interface resolution
    if resolve_interfaces && current_depth == 0
        && let Some(name_indices) = def_idx.name_index.get(&method_lower) {
            for &di in name_indices {
                if node_count.load(std::sync::atomic::Ordering::Relaxed) >= limits.max_total_nodes { break; }
                if let Some(def) = def_idx.definitions.get(di as usize)
                    && let Some(ref parent_class_name) = def.parent {
                        let parent_lower = parent_class_name.to_lowercase();
                        if let Some(parent_indices) = def_idx.name_index.get(&parent_lower) {
                            for &pi in parent_indices {
                                if let Some(parent_def) = def_idx.definitions.get(pi as usize)
                                    && parent_def.kind == DefinitionKind::Interface
                                        && let Some(impl_indices) = def_idx.base_type_index.get(&parent_lower) {
                                            for &ii in impl_indices {
                                                if let Some(impl_def) = def_idx.definitions.get(ii as usize)
                                                    && (impl_def.kind == DefinitionKind::Class || impl_def.kind == DefinitionKind::Struct) {
                                                        let impl_callers = build_caller_tree(
                                                            method_name,
                                                            Some(&impl_def.name),
                                                            max_depth,
                                                            current_depth + 1,
                                                            content_index,
                                                            def_idx,
                                                            ext_filter,
                                                            exclude_dir,
                                                            exclude_file,
                                                            false,
                                                            visited,
                                                            limits,
                                                            node_count,
                                                        );
                                                        callers.extend(impl_callers);
                                                    }
                                            }
                                        }
                            }
                        }
                    }
            }
        }

    callers
}

/// Build a callee tree (direction = "down"): find what methods are called by this method.
/// Uses pre-computed call graph from AST analysis (method_calls in DefinitionIndex).
fn build_callee_tree(
    method_name: &str,
    class_filter: Option<&str>,
    max_depth: usize,
    current_depth: usize,
    def_idx: &DefinitionIndex,
    ext_filter: &str,
    exclude_dir: &[String],
    exclude_file: &[String],
    visited: &mut std::collections::HashSet<String>,
    limits: &CallerLimits,
    node_count: &std::sync::atomic::AtomicUsize,
) -> Vec<Value> {
    if current_depth >= max_depth {
        return Vec::new();
    }
    if node_count.load(std::sync::atomic::Ordering::Relaxed) >= limits.max_total_nodes {
        return Vec::new();
    }

    let method_lower = method_name.to_lowercase();
    let visit_key = if let Some(cls) = class_filter {
        format!("{}.{}", cls.to_lowercase(), method_lower)
    } else {
        method_lower.clone()
    };
    if !visited.insert(visit_key) {
        return Vec::new();
    }

    // Find all definitions of this method (with their def_idx indices)
    let method_def_indices: Vec<u32> = def_idx.name_index
        .get(&method_lower)
        .map(|indices| {
            indices.iter()
                .filter(|&&di| {
                    def_idx.definitions.get(di as usize)
                        .is_some_and(|d| {
                            let kind_ok = d.kind == DefinitionKind::Method || d.kind == DefinitionKind::Constructor;
                            if !kind_ok { return false; }

                            // Apply class filter: only match methods whose parent matches
                            if let Some(cls) = class_filter {
                                let cls_lower = cls.to_lowercase();
                                match &d.parent {
                                    Some(parent) => parent.to_lowercase() == cls_lower,
                                    None => false,
                                }
                            } else {
                                true
                            }
                        })
                })
                .copied()
                .collect()
        })
        .unwrap_or_default();

    if method_def_indices.is_empty() {
        return Vec::new();
    }

    let mut callees: Vec<Value> = Vec::new();
    let mut seen_callees: std::collections::HashSet<String> = std::collections::HashSet::new();

    for &method_di in &method_def_indices {
        if callees.len() >= limits.max_callers_per_level { break; }
        if node_count.load(std::sync::atomic::Ordering::Relaxed) >= limits.max_total_nodes { break; }

        // Get pre-computed call sites for this method
        let call_sites = match def_idx.method_calls.get(&method_di) {
            Some(calls) => calls,
            None => continue,
        };

        for call in call_sites {
            if callees.len() >= limits.max_callers_per_level { break; }
            if node_count.load(std::sync::atomic::Ordering::Relaxed) >= limits.max_total_nodes { break; }

            // Resolve this call site to actual definitions
            let resolved = resolve_call_site(call, def_idx);

            for callee_di in resolved {
                if callees.len() >= limits.max_callers_per_level { break; }
                if node_count.load(std::sync::atomic::Ordering::Relaxed) >= limits.max_total_nodes { break; }

                let callee_def = match def_idx.definitions.get(callee_di as usize) {
                    Some(d) => d,
                    None => continue,
                };

                let callee_file = def_idx.files.get(callee_def.file_id as usize)
                    .map(|s| s.as_str()).unwrap_or("");

                // Apply extension filter
                let matches_ext = Path::new(callee_file)
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case(ext_filter));
                if !matches_ext { continue; }

                // Apply directory/file exclusions
                let path_lower = callee_file.to_lowercase();
                if exclude_dir.iter().any(|excl| path_lower.contains(&excl.to_lowercase())) { continue; }
                if exclude_file.iter().any(|excl| path_lower.contains(&excl.to_lowercase())) { continue; }

                let callee_key = format!("{}.{}",
                    callee_def.parent.as_deref().unwrap_or("?"),
                    &callee_def.name
                );

                if seen_callees.contains(&callee_key) { continue; }
                seen_callees.insert(callee_key.clone());

                node_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                let sub_callees = build_callee_tree(
                    &callee_def.name,
                    None, // don't propagate class filter to sub-callees
                    max_depth,
                    current_depth + 1,
                    def_idx,
                    ext_filter,
                    exclude_dir,
                    exclude_file,
                    visited,
                    limits,
                    node_count,
                );

                let mut node = json!({
                    "method": callee_def.name,
                    "line": callee_def.line_start,
                    "callSiteLine": call.line,
                });
                if let Some(ref parent) = callee_def.parent {
                    node["class"] = json!(parent);
                }
                if let Some(fname) = Path::new(callee_file).file_name().and_then(|f| f.to_str()) {
                    node["file"] = json!(fname);
                }
                if let Some(ref recv) = call.receiver_type {
                    node["receiverType"] = json!(recv);
                }
                if !sub_callees.is_empty() {
                    node["callees"] = json!(sub_callees);
                }
                callees.push(node);
            }
        }
    }

    callees
}

/// Resolve a CallSite to actual definition indices in the definition index.
/// Uses receiver_type to disambiguate when available, and falls back to
/// name-only matching when receiver is unknown.
fn resolve_call_site(call: &CallSite, def_idx: &DefinitionIndex) -> Vec<u32> {
    let name_lower = call.method_name.to_lowercase();
    let candidates = match def_idx.name_index.get(&name_lower) {
        Some(c) => c,
        None => return Vec::new(),
    };

    let mut resolved: Vec<u32> = Vec::new();

    for &di in candidates {
        let def = match def_idx.definitions.get(di as usize) {
            Some(d) => d,
            None => continue,
        };

        // Only match methods and constructors
        if def.kind != DefinitionKind::Method && def.kind != DefinitionKind::Constructor {
            continue;
        }

        if let Some(ref recv_type) = call.receiver_type {
            // We have receiver type info â€” use it to disambiguate
            let recv_lower = recv_type.to_lowercase();

            if let Some(ref parent) = def.parent {
                let parent_lower = parent.to_lowercase();

                // Direct match: parent class name == receiver type
                if parent_lower == recv_lower {
                    resolved.push(di);
                    continue;
                }

                // Interface match: receiver is an interface, parent implements it
                // Check if parent's class definition has recv_type in base_types
                if let Some(parent_defs) = def_idx.name_index.get(&parent_lower) {
                    for &pi in parent_defs {
                        if let Some(parent_def) = def_idx.definitions.get(pi as usize) {
                            if matches!(parent_def.kind,
                                DefinitionKind::Class | DefinitionKind::Struct | DefinitionKind::Record)
                            {
                                let implements = parent_def.base_types.iter()
                                    .any(|bt| {
                                        let bt_base = bt.split('<').next().unwrap_or(bt);
                                        bt_base.eq_ignore_ascii_case(&recv_lower)
                                    });
                                if implements {
                                    resolved.push(di);
                                    break;
                                }
                            }
                        }
                    }
                }

                // Also check: is the receiver type itself a class/struct that this method belongs to?
                // (for cases where receiver_type is a concrete class, not interface)
                // This is already handled by the direct match above.
            }
        } else {
            // No receiver type â€” accept any matching method/constructor
            // (this handles simple calls like Foo() within the same class)
            resolved.push(di);
        }
    }

    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Posting;
    use crate::TrigramIndex;

    #[test]
    fn test_tool_definitions_count() {
        let tools = tool_definitions();
        assert_eq!(tools.len(), 8);
    }

    #[test]
    fn test_tool_definitions_names() {
        let tools = tool_definitions();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"search_grep"));
        assert!(names.contains(&"search_find"));
        assert!(names.contains(&"search_fast"));
        assert!(names.contains(&"search_info"));
        assert!(names.contains(&"search_reindex"));
        assert!(names.contains(&"search_reindex_definitions"));
        assert!(names.contains(&"search_definitions"));
        assert!(names.contains(&"search_callers"));
    }

    #[test]
    fn test_tool_definitions_have_schemas() {
        let tools = tool_definitions();
        for tool in &tools {
            assert!(tool.input_schema.is_object(), "Tool {} should have an object schema", tool.name);
            assert_eq!(tool.input_schema["type"], "object");
        }
    }

    #[test]
    fn test_search_grep_required_fields() {
        let tools = tool_definitions();
        let grep = tools.iter().find(|t| t.name == "search_grep").unwrap();
        let required = grep.input_schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "terms");
    }

    #[test]
    fn test_search_find_has_slow_warning() {
        let tools = tool_definitions();
        let find = tools.iter().find(|t| t.name == "search_find").unwrap();
        assert!(find.description.contains("WARNING"), "search_find should have slow warning");
    }

    #[test]
    fn test_dispatch_unknown_tool() {
        let index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec![],
            index: HashMap::new(),
            total_tokens: 0,
            extensions: vec![],
            file_token_counts: vec![],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: None,
            path_to_id: None,
        };
        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(index)),
            def_index: None,
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),
        };
        let result = dispatch_tool(&ctx, "nonexistent_tool", &json!({}));
        assert!(result.is_error);
        assert!(result.content[0].text.contains("Unknown tool"));
    }

    #[test]
    fn test_dispatch_grep_missing_terms() {
        let index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec![],
            index: HashMap::new(),
            total_tokens: 0,
            extensions: vec![],
            file_token_counts: vec![],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: None,
            path_to_id: None,
        };
        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(index)),
            def_index: None,
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),
        };
        let result = dispatch_tool(&ctx, "search_grep", &json!({}));
        assert!(result.is_error);
        assert!(result.content[0].text.contains("Missing required parameter: terms"));
    }

    #[test]
    fn test_dispatch_grep_empty_index() {
        let index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec![],
            index: HashMap::new(),
            total_tokens: 0,
            extensions: vec![],
            file_token_counts: vec![],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: None,
            path_to_id: None,
        };
        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(index)),
            def_index: None,
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),
        };
        let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "HttpClient"}));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["totalFiles"], 0);
    }

    #[test]
    fn test_dispatch_grep_with_results() {
        let mut idx = HashMap::new();
        idx.insert("httpclient".to_string(), vec![Posting {
            file_id: 0,
            lines: vec![5, 12],
        }]);
        let index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec!["C:\\test\\Program.cs".to_string()],
            index: idx,
            total_tokens: 100,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![50],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: None,
            path_to_id: None,
        };
        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(index)),
            def_index: None,
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),
        };
        let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "HttpClient"}));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["totalFiles"], 1);
        assert_eq!(output["files"][0]["path"], "C:\\test\\Program.cs");
        assert_eq!(output["files"][0]["occurrences"], 2);
    }

    // â”€â”€â”€ Helper: create a context with both content + definition indexes â”€â”€â”€

    fn make_ctx_with_defs() -> HandlerContext {
        use crate::definitions::*;
        use std::path::PathBuf;

        // Content index: tokens -> files+lines
        let mut content_idx = HashMap::new();
        content_idx.insert("executequeryasync".to_string(), vec![
            Posting { file_id: 0, lines: vec![242] },  // ResilientClient.cs:242 (definition)
            Posting { file_id: 1, lines: vec![88] },    // ProxyClient.cs:88 (calls it)
            Posting { file_id: 2, lines: vec![391] },   // QueryService.cs:391 (calls it)
        ]);
        content_idx.insert("queryinternalasync".to_string(), vec![
            Posting { file_id: 2, lines: vec![766] },   // definition
            Posting { file_id: 2, lines: vec![462] },   // called from QueryImplAsync
        ]);
        content_idx.insert("proxyclient".to_string(), vec![
            Posting { file_id: 1, lines: vec![1, 88] },
        ]);
        content_idx.insert("resilientclient".to_string(), vec![
            Posting { file_id: 0, lines: vec![1, 242] },
        ]);
        content_idx.insert("queryservice".to_string(), vec![
            Posting { file_id: 2, lines: vec![1, 391, 462, 766] },
        ]);

        let content_index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec![
                "C:\\src\\ResilientClient.cs".to_string(),
                "C:\\src\\ProxyClient.cs".to_string(),
                "C:\\src\\QueryService.cs".to_string(),
            ],
            index: content_idx,
            total_tokens: 500,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![100, 50, 200],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: None,
            path_to_id: None,
        };

        // Definition index: file -> definitions with line ranges
        let definitions = vec![
            // file 0: ResilientClient
            DefinitionEntry {
                file_id: 0, name: "ResilientClient".to_string(),
                kind: DefinitionKind::Class, line_start: 1, line_end: 300,
                parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            DefinitionEntry {
                file_id: 0, name: "ExecuteQueryAsync".to_string(),
                kind: DefinitionKind::Method, line_start: 240, line_end: 260,
                parent: Some("ResilientClient".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            // file 1: ProxyClient
            DefinitionEntry {
                file_id: 1, name: "ProxyClient".to_string(),
                kind: DefinitionKind::Class, line_start: 1, line_end: 100,
                parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            DefinitionEntry {
                file_id: 1, name: "ExecuteQueryAsync".to_string(),
                kind: DefinitionKind::Method, line_start: 85, line_end: 95,
                parent: Some("ProxyClient".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            // file 2: QueryService
            DefinitionEntry {
                file_id: 2, name: "QueryService".to_string(),
                kind: DefinitionKind::Class, line_start: 1, line_end: 900,
                parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            DefinitionEntry {
                file_id: 2, name: "RunQueryBatchAsync".to_string(),
                kind: DefinitionKind::Method, line_start: 386, line_end: 395,
                parent: Some("QueryService".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            DefinitionEntry {
                file_id: 2, name: "QueryImplAsync".to_string(),
                kind: DefinitionKind::Method, line_start: 450, line_end: 470,
                parent: Some("QueryService".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            DefinitionEntry {
                file_id: 2, name: "QueryInternalAsync".to_string(),
                kind: DefinitionKind::Method, line_start: 760, line_end: 830,
                parent: Some("QueryService".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
        ];

        let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
        let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
        let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();

        for (i, def) in definitions.iter().enumerate() {
            let idx = i as u32;
            name_index.entry(def.name.to_lowercase()).or_default().push(idx);
            kind_index.entry(def.kind.clone()).or_default().push(idx);
            file_index.entry(def.file_id).or_default().push(idx);
        }

        path_to_id.insert(PathBuf::from("C:\\src\\ResilientClient.cs"), 0);
        path_to_id.insert(PathBuf::from("C:\\src\\ProxyClient.cs"), 1);
        path_to_id.insert(PathBuf::from("C:\\src\\QueryService.cs"), 2);

        let def_index = DefinitionIndex {
            root: ".".to_string(),
            created_at: 0,
            extensions: vec!["cs".to_string()],
            files: vec![
                "C:\\src\\ResilientClient.cs".to_string(),
                "C:\\src\\ProxyClient.cs".to_string(),
                "C:\\src\\QueryService.cs".to_string(),
            ],
            definitions,
            name_index,
            kind_index,
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index,
            path_to_id,
            method_calls: HashMap::new(),
        };

        HandlerContext {
            index: Arc::new(RwLock::new(content_index)),
            def_index: Some(Arc::new(RwLock::new(def_index))),
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),
        }
    }

    // â”€â”€â”€ search_callers tests â”€â”€â”€

    #[test]
    fn test_search_callers_missing_method() {
        let ctx = make_ctx_with_defs();
        let result = dispatch_tool(&ctx, "search_callers", &json!({}));
        assert!(result.is_error);
        assert!(result.content[0].text.contains("Missing required parameter: method"));
    }

    #[test]
    fn test_search_callers_no_def_index() {
        let index = ContentIndex {
            root: ".".to_string(), created_at: 0, max_age_secs: 3600,
            files: vec![], index: HashMap::new(), total_tokens: 0,
            extensions: vec![], file_token_counts: vec![],
            trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
        };
        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(index)),
            def_index: None,
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),
        };
        let result = dispatch_tool(&ctx, "search_callers", &json!({"method": "Foo"}));
        assert!(result.is_error);
        assert!(result.content[0].text.contains("Definition index not available"));
    }

    #[test]
    fn test_search_callers_finds_callers() {
        let ctx = make_ctx_with_defs();
        let result = dispatch_tool(&ctx, "search_callers", &json!({
            "method": "ExecuteQueryAsync",
            "depth": 2
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let tree = output["callTree"].as_array().unwrap();
        // Should find callers (ProxyClient.ExecuteQueryAsync and QueryService.RunQueryBatchAsync)
        assert!(!tree.is_empty(), "Call tree should not be empty");
        // Check that summary exists
        assert!(output["summary"]["totalNodes"].as_u64().unwrap() > 0);
        assert!(output["summary"]["searchTimeMs"].as_f64().is_some());
    }

    #[test]
    fn test_search_callers_nonexistent_method() {
        let ctx = make_ctx_with_defs();
        let result = dispatch_tool(&ctx, "search_callers", &json!({
            "method": "NonExistentMethodXYZ"
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let tree = output["callTree"].as_array().unwrap();
        assert!(tree.is_empty(), "Call tree should be empty for nonexistent method");
    }

    #[test]
    fn test_search_callers_max_total_nodes() {
        let ctx = make_ctx_with_defs();
        let result = dispatch_tool(&ctx, "search_callers", &json!({
            "method": "ExecuteQueryAsync",
            "depth": 5,
            "maxTotalNodes": 2
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let total = output["summary"]["totalNodes"].as_u64().unwrap();
        assert!(total <= 2, "Total nodes should be capped at 2, got {}", total);
    }

    #[test]
    fn test_search_callers_max_per_level() {
        let ctx = make_ctx_with_defs();
        let result = dispatch_tool(&ctx, "search_callers", &json!({
            "method": "ExecuteQueryAsync",
            "depth": 1,
            "maxCallersPerLevel": 1
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let tree = output["callTree"].as_array().unwrap();
        assert!(tree.len() <= 1, "Should have at most 1 caller per level, got {}", tree.len());
    }

    #[test]
    fn test_search_callers_has_class_and_file() {
        let ctx = make_ctx_with_defs();
        let result = dispatch_tool(&ctx, "search_callers", &json!({
            "method": "ExecuteQueryAsync",
            "depth": 1
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let tree = output["callTree"].as_array().unwrap();
        // Each node should have method, file, and optionally class
        for node in tree {
            assert!(node["method"].is_string(), "Node should have method name");
            assert!(node["file"].is_string(), "Node should have file name");
            assert!(node["line"].is_number(), "Node should have line number");
        }
    }

    // â”€â”€â”€ search_reindex_definitions tests â”€â”€â”€

    #[test]
    fn test_reindex_definitions_no_def_index() {
        let index = ContentIndex {
            root: ".".to_string(), created_at: 0, max_age_secs: 3600,
            files: vec![], index: HashMap::new(), total_tokens: 0,
            extensions: vec![], file_token_counts: vec![],
            trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
        };
        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(index)),
            def_index: None,
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),
        };
        let result = dispatch_tool(&ctx, "search_reindex_definitions", &json!({}));
        assert!(result.is_error);
        assert!(result.content[0].text.contains("Definition index not available"));
    }

    #[test]
    fn test_reindex_definitions_has_schema() {
        let tools = tool_definitions();
        let tool = tools.iter().find(|t| t.name == "search_reindex_definitions").unwrap();
        let props = tool.input_schema["properties"].as_object().unwrap();
        assert!(props.contains_key("dir"), "Should have dir parameter");
        assert!(props.contains_key("ext"), "Should have ext parameter");
    }

    // â”€â”€â”€ containsLine tests â”€â”€â”€

    #[test]
    fn test_contains_line_requires_file() {
        let ctx = make_ctx_with_defs();
        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "containsLine": 391
        }));
        assert!(result.is_error);
        assert!(result.content[0].text.contains("containsLine requires 'file' parameter"));
    }

    #[test]
    fn test_contains_line_finds_method() {
        let ctx = make_ctx_with_defs();
        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "file": "QueryService",
            "containsLine": 391
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let defs = output["containingDefinitions"].as_array().unwrap();
        // Line 391 is inside RunQueryBatchAsync (386-395) and QueryService (1-900)
        assert!(!defs.is_empty(), "Should find containing definitions");
        // Most specific (smallest range) should be first
        assert_eq!(defs[0]["name"], "RunQueryBatchAsync");
        assert_eq!(defs[0]["kind"], "method");
    }

    #[test]
    fn test_contains_line_returns_parent() {
        let ctx = make_ctx_with_defs();
        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "file": "QueryService",
            "containsLine": 800
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let defs = output["containingDefinitions"].as_array().unwrap();
        // Line 800 is inside QueryInternalAsync (760-830)
        let method = defs.iter().find(|d| d["kind"] == "method").unwrap();
        assert_eq!(method["name"], "QueryInternalAsync");
        assert_eq!(method["parent"], "QueryService");
    }

    #[test]
    fn test_contains_line_no_match() {
        let ctx = make_ctx_with_defs();
        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "file": "QueryService",
            "containsLine": 999
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let defs = output["containingDefinitions"].as_array().unwrap();
        // Line 999 is outside all definitions (class ends at 900)
        assert!(defs.is_empty(), "Should find no definitions for line 999");
    }

    // â”€â”€â”€ find_containing_method tests â”€â”€â”€

    #[test]
    fn test_find_containing_method_innermost() {
        let ctx = make_ctx_with_defs();
        let def_idx = ctx.def_index.as_ref().unwrap().read().unwrap();
        // Line 391 is inside RunQueryBatchAsync (386-395) which is inside QueryService (1-900)
        let result = find_containing_method(&def_idx, 2, 391);
        assert!(result.is_some());
        let (name, parent, _line) = result.unwrap();
        assert_eq!(name, "RunQueryBatchAsync");
        assert_eq!(parent.as_deref(), Some("QueryService"));
    }

    #[test]
    fn test_find_containing_method_none() {
        let ctx = make_ctx_with_defs();
        let def_idx = ctx.def_index.as_ref().unwrap().read().unwrap();
        // Line 999 is outside all methods
        let result = find_containing_method(&def_idx, 2, 999);
        assert!(result.is_none());
    }

    // â”€â”€â”€ search_callers schema tests â”€â”€â”€

    #[test]
    fn test_search_callers_has_required_params() {
        let tools = tool_definitions();
        let callers = tools.iter().find(|t| t.name == "search_callers").unwrap();
        let required = callers.input_schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "method");
    }

    #[test]
    fn test_search_callers_has_limit_params() {
        let tools = tool_definitions();
        let callers = tools.iter().find(|t| t.name == "search_callers").unwrap();
        let props = callers.input_schema["properties"].as_object().unwrap();
        assert!(props.contains_key("maxCallersPerLevel"), "Should have maxCallersPerLevel");
        assert!(props.contains_key("maxTotalNodes"), "Should have maxTotalNodes");
    }

    #[test]
    fn test_search_definitions_has_contains_line() {
        let tools = tool_definitions();
        let defs = tools.iter().find(|t| t.name == "search_definitions").unwrap();
        let props = defs.input_schema["properties"].as_object().unwrap();
        assert!(props.contains_key("containsLine"), "Should have containsLine parameter");
    }
    // â”€â”€â”€ resolve_call_site tests â”€â”€â”€

    #[test]
    fn test_resolve_call_site_with_class_scope() {
        use crate::definitions::*;

        // Build a definition index with two classes, each having a method with the same name
        let definitions = vec![
            // Class A
            DefinitionEntry {
                file_id: 0, name: "ServiceA".to_string(),
                kind: DefinitionKind::Class, line_start: 1, line_end: 50,
                parent: None, signature: None, modifiers: vec![], attributes: vec![],
                base_types: vec!["IService".to_string()],
            },
            DefinitionEntry {
                file_id: 0, name: "Execute".to_string(),
                kind: DefinitionKind::Method, line_start: 10, line_end: 20,
                parent: Some("ServiceA".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            // Class B
            DefinitionEntry {
                file_id: 1, name: "ServiceB".to_string(),
                kind: DefinitionKind::Class, line_start: 1, line_end: 50,
                parent: None, signature: None, modifiers: vec![], attributes: vec![],
                base_types: vec![],
            },
            DefinitionEntry {
                file_id: 1, name: "Execute".to_string(),
                kind: DefinitionKind::Method, line_start: 10, line_end: 20,
                parent: Some("ServiceB".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
        ];

        let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
        let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
        let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut base_type_index: HashMap<String, Vec<u32>> = HashMap::new();

        for (i, def) in definitions.iter().enumerate() {
            let idx = i as u32;
            name_index.entry(def.name.to_lowercase()).or_default().push(idx);
            kind_index.entry(def.kind.clone()).or_default().push(idx);
            file_index.entry(def.file_id).or_default().push(idx);
            for bt in &def.base_types {
                base_type_index.entry(bt.to_lowercase()).or_default().push(idx);
            }
        }

        let def_index = DefinitionIndex {
            root: ".".to_string(),
            created_at: 0,
            extensions: vec!["cs".to_string()],
            files: vec!["a.cs".to_string(), "b.cs".to_string()],
            definitions,
            name_index,
            kind_index,
            attribute_index: HashMap::new(),
            base_type_index,
            file_index,
            path_to_id: HashMap::new(),
            method_calls: HashMap::new(),
        };

        // Case 1: Call with receiver_type = "ServiceA" should resolve to ServiceA.Execute only
        let call_a = CallSite {
            method_name: "Execute".to_string(),
            receiver_type: Some("ServiceA".to_string()),
            line: 5,
        };
        let resolved_a = resolve_call_site(&call_a, &def_index);
        assert_eq!(resolved_a.len(), 1, "Should resolve to exactly one definition for ServiceA.Execute");
        assert_eq!(def_index.definitions[resolved_a[0] as usize].parent.as_deref(), Some("ServiceA"));

        // Case 2: Call with receiver_type = "ServiceB" should resolve to ServiceB.Execute only
        let call_b = CallSite {
            method_name: "Execute".to_string(),
            receiver_type: Some("ServiceB".to_string()),
            line: 10,
        };
        let resolved_b = resolve_call_site(&call_b, &def_index);
        assert_eq!(resolved_b.len(), 1, "Should resolve to exactly one definition for ServiceB.Execute");
        assert_eq!(def_index.definitions[resolved_b[0] as usize].parent.as_deref(), Some("ServiceB"));

        // Case 3: Call with no receiver_type should resolve to BOTH Execute methods
        let call_no_recv = CallSite {
            method_name: "Execute".to_string(),
            receiver_type: None,
            line: 15,
        };
        let resolved_none = resolve_call_site(&call_no_recv, &def_index);
        assert_eq!(resolved_none.len(), 2, "No receiver should match all Execute methods");

        // Case 4: Call with receiver_type = "IService" (interface) should resolve to
        // ServiceA.Execute because ServiceA implements IService
        let call_iface = CallSite {
            method_name: "Execute".to_string(),
            receiver_type: Some("IService".to_string()),
            line: 20,
        };
        let resolved_iface = resolve_call_site(&call_iface, &def_index);
        assert!(!resolved_iface.is_empty(), "Interface receiver should resolve to implementing class method");
        assert!(resolved_iface.iter().any(|&di| {
            def_index.definitions[di as usize].parent.as_deref() == Some("ServiceA")
        }), "Should resolve IService.Execute to ServiceA.Execute");
    }

    // â”€â”€â”€ search_callers "down" direction + class filter tests â”€â”€â”€

    #[test]
    fn test_search_callers_down_class_filter() {
        use crate::definitions::*;
        use std::path::PathBuf;

        // Two classes with same method name "SearchInternalAsync",
        // each calling different methods.
        let definitions = vec![
            // Class: IndexSearchService (file 0)
            DefinitionEntry {
                file_id: 0, name: "IndexSearchService".to_string(),
                kind: DefinitionKind::Class, line_start: 1, line_end: 900,
                parent: None, signature: None, modifiers: vec![], attributes: vec![],
                base_types: vec![],
            },
            // di=1
            DefinitionEntry {
                file_id: 0, name: "SearchInternalAsync".to_string(),
                kind: DefinitionKind::Method, line_start: 766, line_end: 833,
                parent: Some("IndexSearchService".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            // di=2: callee of IndexSearchService.SearchInternalAsync
            DefinitionEntry {
                file_id: 0, name: "ShouldIssueVectorSearch".to_string(),
                kind: DefinitionKind::Method, line_start: 200, line_end: 220,
                parent: Some("IndexSearchService".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            // Class: IndexedSearchQueryExecuter (file 1)
            DefinitionEntry {
                file_id: 1, name: "IndexedSearchQueryExecuter".to_string(),
                kind: DefinitionKind::Class, line_start: 1, line_end: 400,
                parent: None, signature: None, modifiers: vec![], attributes: vec![],
                base_types: vec![],
            },
            // di=4
            DefinitionEntry {
                file_id: 1, name: "SearchInternalAsync".to_string(),
                kind: DefinitionKind::Method, line_start: 328, line_end: 341,
                parent: Some("IndexedSearchQueryExecuter".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            // di=5: callee of IndexedSearchQueryExecuter.SearchInternalAsync
            DefinitionEntry {
                file_id: 1, name: "TraceInformation".to_string(),
                kind: DefinitionKind::Method, line_start: 50, line_end: 55,
                parent: Some("IndexedSearchQueryExecuter".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
        ];

        let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
        let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
        let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();

        for (i, def) in definitions.iter().enumerate() {
            let idx = i as u32;
            name_index.entry(def.name.to_lowercase()).or_default().push(idx);
            kind_index.entry(def.kind.clone()).or_default().push(idx);
            file_index.entry(def.file_id).or_default().push(idx);
        }

        // method_calls: map method def_idx -> vec of CallSite
        let mut method_calls: HashMap<u32, Vec<CallSite>> = HashMap::new();

        // IndexSearchService.SearchInternalAsync (di=1) calls ShouldIssueVectorSearch
        method_calls.insert(1, vec![
            CallSite { method_name: "ShouldIssueVectorSearch".to_string(), receiver_type: None, line: 780 },
        ]);
        // IndexedSearchQueryExecuter.SearchInternalAsync (di=4) calls TraceInformation
        method_calls.insert(4, vec![
            CallSite { method_name: "TraceInformation".to_string(), receiver_type: None, line: 333 },
        ]);

        let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();
        path_to_id.insert(PathBuf::from("C:\\src\\IndexSearchService.cs"), 0);
        path_to_id.insert(PathBuf::from("C:\\src\\IndexedSearchQueryExecuter.cs"), 1);

        let def_index = DefinitionIndex {
            root: ".".to_string(),
            created_at: 0,
            extensions: vec!["cs".to_string()],
            files: vec![
                "C:\\src\\IndexSearchService.cs".to_string(),
                "C:\\src\\IndexedSearchQueryExecuter.cs".to_string(),
            ],
            definitions,
            name_index,
            kind_index,
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index,
            path_to_id,
            method_calls,
        };

        let content_index = ContentIndex {
            root: ".".to_string(), created_at: 0, max_age_secs: 3600,
            files: vec![
                "C:\\src\\IndexSearchService.cs".to_string(),
                "C:\\src\\IndexedSearchQueryExecuter.cs".to_string(),
            ],
            index: HashMap::new(), total_tokens: 0,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![100, 100],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: None, path_to_id: None,
        };

        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(content_index)),
            def_index: Some(Arc::new(RwLock::new(def_index))),
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),
        };

        // Test 1: direction=down with class=IndexSearchService
        // Should find ShouldIssueVectorSearch, NOT TraceInformation
        let result = dispatch_tool(&ctx, "search_callers", &json!({
            "method": "SearchInternalAsync",
            "class": "IndexSearchService",
            "direction": "down",
            "depth": 1
        }));
        assert!(!result.is_error, "Should succeed");
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let tree = output["callTree"].as_array().unwrap();
        assert!(!tree.is_empty(), "Should have callees for IndexSearchService.SearchInternalAsync");

        let callee_names: Vec<&str> = tree.iter()
            .filter_map(|n| n["method"].as_str())
            .collect();
        assert!(callee_names.contains(&"ShouldIssueVectorSearch"),
            "Should contain ShouldIssueVectorSearch, got: {:?}", callee_names);
        assert!(!callee_names.contains(&"TraceInformation"),
            "Should NOT contain TraceInformation from IndexedSearchQueryExecuter, got: {:?}", callee_names);

        // Verify class appears in query output
        assert_eq!(output["query"]["class"], "IndexSearchService");

        // Test 2: direction=down with class=IndexedSearchQueryExecuter
        // Should find TraceInformation, NOT ShouldIssueVectorSearch
        let result2 = dispatch_tool(&ctx, "search_callers", &json!({
            "method": "SearchInternalAsync",
            "class": "IndexedSearchQueryExecuter",
            "direction": "down",
            "depth": 1
        }));
        assert!(!result2.is_error);
        let output2: Value = serde_json::from_str(&result2.content[0].text).unwrap();
        let tree2 = output2["callTree"].as_array().unwrap();

        let callee_names2: Vec<&str> = tree2.iter()
            .filter_map(|n| n["method"].as_str())
            .collect();
        assert!(callee_names2.contains(&"TraceInformation"),
            "Should contain TraceInformation, got: {:?}", callee_names2);
        assert!(!callee_names2.contains(&"ShouldIssueVectorSearch"),
            "Should NOT contain ShouldIssueVectorSearch, got: {:?}", callee_names2);

        // Test 3: direction=down WITHOUT class filter â†’ should get callees from BOTH classes
        let result3 = dispatch_tool(&ctx, "search_callers", &json!({
            "method": "SearchInternalAsync",
            "direction": "down",
            "depth": 1
        }));
        assert!(!result3.is_error);
        let output3: Value = serde_json::from_str(&result3.content[0].text).unwrap();
        let tree3 = output3["callTree"].as_array().unwrap();

        let callee_names3: Vec<&str> = tree3.iter()
            .filter_map(|n| n["method"].as_str())
            .collect();
        assert!(callee_names3.contains(&"ShouldIssueVectorSearch"),
            "Without class filter, should find ShouldIssueVectorSearch, got: {:?}", callee_names3);
        assert!(callee_names3.contains(&"TraceInformation"),
            "Without class filter, should find TraceInformation, got: {:?}", callee_names3);

        // Verify ambiguity warning is present when no class filter is specified
        assert!(output3.get("warning").is_some(),
            "Should have ambiguity warning when no class filter and multiple classes have same method");
    }

    // â”€â”€â”€ includeBody tests â”€â”€â”€

    /// Helper: create temp files with known content + build HandlerContext with DefinitionIndex pointing to them
    fn make_ctx_with_real_files() -> (HandlerContext, std::path::PathBuf) {
        use crate::definitions::*;
        use std::path::PathBuf;
        use std::io::Write;

        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tmp_dir = std::env::temp_dir().join(format!("search_test_{}_{}", std::process::id(), id));
        let _ = std::fs::create_dir_all(&tmp_dir);

        // File 0: MyService.cs â€” 15 lines
        let file0_path = tmp_dir.join("MyService.cs");
        {
            let mut f = std::fs::File::create(&file0_path).unwrap();
            for i in 1..=15 {
                writeln!(f, "// line {}", i).unwrap();
            }
        }

        // File 1: BigFile.cs â€” 25 lines
        let file1_path = tmp_dir.join("BigFile.cs");
        {
            let mut f = std::fs::File::create(&file1_path).unwrap();
            for i in 1..=25 {
                writeln!(f, "// big line {}", i).unwrap();
            }
        }

        let file0_str = file0_path.to_string_lossy().to_string();
        let file1_str = file1_path.to_string_lossy().to_string();

        let definitions = vec![
            // file 0: class MyService lines 1-15
            DefinitionEntry {
                file_id: 0, name: "MyService".to_string(),
                kind: DefinitionKind::Class, line_start: 1, line_end: 15,
                parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            // file 0: method DoWork lines 3-8
            DefinitionEntry {
                file_id: 0, name: "DoWork".to_string(),
                kind: DefinitionKind::Method, line_start: 3, line_end: 8,
                parent: Some("MyService".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            // file 1: class BigClass lines 1-25
            DefinitionEntry {
                file_id: 1, name: "BigClass".to_string(),
                kind: DefinitionKind::Class, line_start: 1, line_end: 25,
                parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
            },
            // file 1: method Process lines 5-24 (20 lines)
            DefinitionEntry {
                file_id: 1, name: "Process".to_string(),
                kind: DefinitionKind::Method, line_start: 5, line_end: 24,
                parent: Some("BigClass".to_string()), signature: None,
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
        ];

        let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
        let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
        let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();

        for (i, def) in definitions.iter().enumerate() {
            let idx = i as u32;
            name_index.entry(def.name.to_lowercase()).or_default().push(idx);
            kind_index.entry(def.kind.clone()).or_default().push(idx);
            file_index.entry(def.file_id).or_default().push(idx);
        }

        path_to_id.insert(file0_path.clone(), 0);
        path_to_id.insert(file1_path.clone(), 1);

        let def_index = DefinitionIndex {
            root: tmp_dir.to_string_lossy().to_string(),
            created_at: 0,
            extensions: vec!["cs".to_string()],
            files: vec![file0_str.clone(), file1_str.clone()],
            definitions,
            name_index,
            kind_index,
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index,
            path_to_id,
            method_calls: HashMap::new(),
        };

        let content_idx = HashMap::new();
        let content_index = ContentIndex {
            root: tmp_dir.to_string_lossy().to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec![file0_str, file1_str],
            index: content_idx,
            total_tokens: 0,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![0, 0],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: None,
            path_to_id: None,
        };

        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(content_index)),
            def_index: Some(Arc::new(RwLock::new(def_index))),
            server_dir: tmp_dir.to_string_lossy().to_string(),
            server_ext: "cs".to_string(),
        };

        (ctx, tmp_dir)
    }

    fn cleanup_tmp(tmp_dir: &std::path::Path) {
        let _ = std::fs::remove_dir_all(tmp_dir);
    }

    #[test]
    fn test_search_definitions_include_body() {
        let (ctx, tmp_dir) = make_ctx_with_real_files();
        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "name": "DoWork",
            "includeBody": true
        }));
        assert!(!result.is_error, "Should not error: {}", result.content[0].text);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let defs = output["definitions"].as_array().unwrap();
        assert_eq!(defs.len(), 1);
        let body = defs[0]["body"].as_array().unwrap();
        // DoWork is lines 3-8, so 6 lines
        assert_eq!(body.len(), 6);
        assert_eq!(defs[0]["bodyStartLine"], 3);
        assert_eq!(body[0], "// line 3");
        assert_eq!(body[5], "// line 8");
        // Should not be truncated
        assert!(defs[0].get("bodyTruncated").is_none());
        // Summary should have totalBodyLinesReturned
        assert_eq!(output["summary"]["totalBodyLinesReturned"], 6);
        cleanup_tmp(&tmp_dir);
    }

    #[test]
    fn test_search_definitions_include_body_default_false() {
        let (ctx, tmp_dir) = make_ctx_with_real_files();
        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "name": "DoWork"
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let defs = output["definitions"].as_array().unwrap();
        assert_eq!(defs.len(), 1);
        // No body field when includeBody is not set
        assert!(defs[0].get("body").is_none(), "body should not be present by default");
        // No totalBodyLinesReturned in summary
        assert!(output["summary"].get("totalBodyLinesReturned").is_none());
        cleanup_tmp(&tmp_dir);
    }

    #[test]
    fn test_search_definitions_max_body_lines_truncation() {
        let (ctx, tmp_dir) = make_ctx_with_real_files();
        // Process method has 20 lines (5-24), request maxBodyLines=5
        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "name": "Process",
            "includeBody": true,
            "maxBodyLines": 5
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let defs = output["definitions"].as_array().unwrap();
        assert_eq!(defs.len(), 1);
        let body = defs[0]["body"].as_array().unwrap();
        assert_eq!(body.len(), 5, "Should only have 5 lines");
        assert_eq!(defs[0]["bodyTruncated"], true);
        assert_eq!(defs[0]["totalBodyLines"], 20);
        cleanup_tmp(&tmp_dir);
    }

    #[test]
    fn test_search_definitions_max_total_body_lines_budget() {
        let (ctx, tmp_dir) = make_ctx_with_real_files();
        // Search for both methods: DoWork (6 lines) + Process (20 lines)
        // Budget of 10 means DoWork (6 lines) fits, Process only gets 4 lines
        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "name": "DoWork,Process",
            "includeBody": true,
            "maxTotalBodyLines": 10
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let defs = output["definitions"].as_array().unwrap();
        assert_eq!(defs.len(), 2);

        // First def gets body
        let first = &defs[0];
        assert!(first.get("body").is_some(), "First def should have body");
        assert!(first.get("bodyOmitted").is_none(), "First def should not be omitted");

        // Total body lines returned should be <= 10
        let total = output["summary"]["totalBodyLinesReturned"].as_u64().unwrap();
        assert!(total <= 10, "Total body lines should be <= 10, got {}", total);
        cleanup_tmp(&tmp_dir);
    }

    #[test]
    fn test_search_definitions_contains_line_with_body() {
        let (ctx, tmp_dir) = make_ctx_with_real_files();
        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "file": "MyService",
            "containsLine": 5,
            "includeBody": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let defs = output["containingDefinitions"].as_array().unwrap();
        assert!(!defs.is_empty(), "Should find containing definitions");
        // The innermost (DoWork, lines 3-8) should be first
        assert_eq!(defs[0]["name"], "DoWork");
        let body = defs[0]["body"].as_array().unwrap();
        assert!(!body.is_empty(), "Body should be present for containsLine with includeBody");
        assert_eq!(defs[0]["bodyStartLine"], 3);
        // Summary should have totalBodyLinesReturned
        assert!(output["summary"]["totalBodyLinesReturned"].as_u64().unwrap() > 0);
        cleanup_tmp(&tmp_dir);
    }

    #[test]
    fn test_search_definitions_file_cache() {
        let (ctx, tmp_dir) = make_ctx_with_real_files();
        // Search with parent filter returning multiple defs from same file
        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "parent": "MyService",
            "includeBody": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let defs = output["definitions"].as_array().unwrap();
        // Should find DoWork (parent=MyService)
        assert!(!defs.is_empty());
        for def in defs {
            assert!(def.get("body").is_some(), "Each def should have body");
        }
        cleanup_tmp(&tmp_dir);
    }

    #[test]
    fn test_search_definitions_stale_file_warning() {
        use std::io::Write;

        let tmp_dir = std::env::temp_dir().join(format!("search_test_stale_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp_dir);

        // Create file with only 10 lines
        let file_path = tmp_dir.join("Stale.cs");
        {
            let mut f = std::fs::File::create(&file_path).unwrap();
            for i in 1..=10 {
                writeln!(f, "// stale line {}", i).unwrap();
            }
        }

        let file_str = file_path.to_string_lossy().to_string();

        let definitions = vec![
            DefinitionEntry {
                file_id: 0, name: "StaleClass".to_string(),
                kind: DefinitionKind::Class, line_start: 5, line_end: 20, // claims line 20 but file only has 10
                parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
            },
        ];

        let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
        let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
        let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();

        for (i, def) in definitions.iter().enumerate() {
            let idx = i as u32;
            name_index.entry(def.name.to_lowercase()).or_default().push(idx);
            kind_index.entry(def.kind.clone()).or_default().push(idx);
            file_index.entry(def.file_id).or_default().push(idx);
        }

        let def_index = DefinitionIndex {
            root: tmp_dir.to_string_lossy().to_string(),
            created_at: 0,
            extensions: vec!["cs".to_string()],
            files: vec![file_str.clone()],
            definitions,
            name_index,
            kind_index,
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index,
            path_to_id: HashMap::new(),
            method_calls: HashMap::new(),
        };

        let content_index = ContentIndex {
            root: tmp_dir.to_string_lossy().to_string(),
            created_at: 0, max_age_secs: 3600,
            files: vec![file_str],
            index: HashMap::new(),
            total_tokens: 0,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![0],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: None,
            path_to_id: None,
        };

        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(content_index)),
            def_index: Some(Arc::new(RwLock::new(def_index))),
            server_dir: tmp_dir.to_string_lossy().to_string(),
            server_ext: "cs".to_string(),
        };

        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "name": "StaleClass",
            "includeBody": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let defs = output["definitions"].as_array().unwrap();
        assert_eq!(defs.len(), 1);
        assert!(defs[0].get("bodyWarning").is_some(), "Should have bodyWarning for stale file");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_search_definitions_body_error() {
        // Def points to non-existent file
        let definitions = vec![
            DefinitionEntry {
                file_id: 0, name: "GhostClass".to_string(),
                kind: DefinitionKind::Class, line_start: 1, line_end: 10,
                parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
            },
        ];

        let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
        let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
        let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();

        for (i, def) in definitions.iter().enumerate() {
            let idx = i as u32;
            name_index.entry(def.name.to_lowercase()).or_default().push(idx);
            kind_index.entry(def.kind.clone()).or_default().push(idx);
            file_index.entry(def.file_id).or_default().push(idx);
        }

        let non_existent = "C:\\nonexistent\\path\\Ghost.cs".to_string();

        let def_index = DefinitionIndex {
            root: ".".to_string(),
            created_at: 0,
            extensions: vec!["cs".to_string()],
            files: vec![non_existent.clone()],
            definitions,
            name_index,
            kind_index,
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index,
            path_to_id: HashMap::new(),
            method_calls: HashMap::new(),
        };

        let content_index = ContentIndex {
            root: ".".to_string(),
            created_at: 0, max_age_secs: 3600,
            files: vec![non_existent],
            index: HashMap::new(),
            total_tokens: 0,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![0],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: None,
            path_to_id: None,
        };

        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(content_index)),
            def_index: Some(Arc::new(RwLock::new(def_index))),
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),
        };

        let result = dispatch_tool(&ctx, "search_definitions", &json!({
            "name": "GhostClass",
            "includeBody": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let defs = output["definitions"].as_array().unwrap();
        assert_eq!(defs.len(), 1);
        assert!(defs[0].get("bodyError").is_some(), "Should have bodyError for missing file");
        assert_eq!(defs[0]["bodyError"], "failed to read file");
    }

    // â”€â”€â”€ build_grouped_line_content tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn test_grouped_line_content_single_group() {
        let lines_vec = vec!["line0", "line1", "line2", "line3", "line4"];
        let mut lines_to_show = std::collections::BTreeSet::new();
        lines_to_show.insert(1);
        lines_to_show.insert(2);
        lines_to_show.insert(3);
        let mut match_lines_set = std::collections::HashSet::new();
        match_lines_set.insert(2);

        let result = build_grouped_line_content(&lines_to_show, &lines_vec, &match_lines_set);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 1, "Should be one group for consecutive lines");
        assert_eq!(groups[0]["startLine"], 2); // 0-based idx 1 â†’ 1-based line 2
        let lines = groups[0]["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "line1");
        assert_eq!(lines[1], "line2");
        assert_eq!(lines[2], "line3");
        let matches = groups[0]["matchIndices"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], 1); // index 1 within the group
    }

    #[test]
    fn test_grouped_line_content_two_groups() {
        // Lines 1,2 and 5,6 â€” gap between 2 and 5
        let lines_vec = vec!["L0", "L1", "L2", "L3", "L4", "L5", "L6"];
        let mut lines_to_show = std::collections::BTreeSet::new();
        for i in [1, 2, 5, 6] { lines_to_show.insert(i); }
        let mut match_lines_set = std::collections::HashSet::new();
        match_lines_set.insert(1);
        match_lines_set.insert(6);

        let result = build_grouped_line_content(&lines_to_show, &lines_vec, &match_lines_set);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 2, "Should be two groups with a gap");

        // Group 1: lines 1-2 (0-based), startLine=2 (1-based)
        assert_eq!(groups[0]["startLine"], 2);
        assert_eq!(groups[0]["lines"].as_array().unwrap().len(), 2);
        let m0 = groups[0]["matchIndices"].as_array().unwrap();
        assert_eq!(m0.len(), 1);
        assert_eq!(m0[0], 0); // match at index 0

        // Group 2: lines 5-6 (0-based), startLine=6 (1-based)
        assert_eq!(groups[1]["startLine"], 6);
        assert_eq!(groups[1]["lines"].as_array().unwrap().len(), 2);
        let m1 = groups[1]["matchIndices"].as_array().unwrap();
        assert_eq!(m1.len(), 1);
        assert_eq!(m1[0], 1); // match at index 1
    }

    #[test]
    fn test_grouped_line_content_no_matches() {
        // Context-only lines, no matches
        let lines_vec = vec!["A", "B", "C"];
        let mut lines_to_show = std::collections::BTreeSet::new();
        lines_to_show.insert(0);
        lines_to_show.insert(1);
        let match_lines_set = std::collections::HashSet::new(); // empty

        let result = build_grouped_line_content(&lines_to_show, &lines_vec, &match_lines_set);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["startLine"], 1);
        // matchIndices should be absent when empty
        assert!(groups[0].get("matchIndices").is_none(), "matchIndices should be omitted when empty");
    }

    #[test]
    fn test_grouped_line_content_empty() {
        let lines_vec: Vec<&str> = vec![];
        let lines_to_show = std::collections::BTreeSet::new();
        let match_lines_set = std::collections::HashSet::new();

        let result = build_grouped_line_content(&lines_to_show, &lines_vec, &match_lines_set);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 0, "Empty input should produce empty groups");
    }

    #[test]
    fn test_grouped_line_content_multiple_matches_in_group() {
        let lines_vec = vec!["A", "B", "C", "D", "E"];
        let mut lines_to_show = std::collections::BTreeSet::new();
        for i in 0..5 { lines_to_show.insert(i); }
        let mut match_lines_set = std::collections::HashSet::new();
        match_lines_set.insert(1);
        match_lines_set.insert(3);

        let result = build_grouped_line_content(&lines_to_show, &lines_vec, &match_lines_set);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 1);
        let matches = groups[0]["matchIndices"].as_array().unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], 1);
        assert_eq!(matches[1], 3);
    }

    // Note: is_csharp_noise_token tests removed â€” function was replaced by
    // AST-based call extraction which doesn't need noise filtering.

    // â”€â”€â”€ sorted_intersect tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn test_sorted_intersect_basic() {
        assert_eq!(sorted_intersect(&[1, 3, 5, 7], &[2, 3, 5, 8]), vec![3, 5]);
    }

    #[test]
    fn test_sorted_intersect_empty_left() {
        let empty: Vec<u32> = vec![];
        assert_eq!(sorted_intersect(&[], &[1, 2, 3]), empty);
    }

    #[test]
    fn test_sorted_intersect_empty_right() {
        let empty: Vec<u32> = vec![];
        assert_eq!(sorted_intersect(&[1, 2, 3], &[]), empty);
    }

    #[test]
    fn test_sorted_intersect_both_empty() {
        let empty: Vec<u32> = vec![];
        assert_eq!(sorted_intersect(&[], &[]), empty);
    }

    #[test]
    fn test_sorted_intersect_disjoint() {
        let empty: Vec<u32> = vec![];
        assert_eq!(sorted_intersect(&[1, 2, 3], &[4, 5, 6]), empty);
    }

    #[test]
    fn test_sorted_intersect_identical() {
        assert_eq!(sorted_intersect(&[1, 2, 3], &[1, 2, 3]), vec![1, 2, 3]);
    }

    #[test]
    fn test_sorted_intersect_single_match() {
        assert_eq!(sorted_intersect(&[1, 5, 10], &[3, 5, 8]), vec![5]);
    }

    // â”€â”€â”€ Substring search handler integration tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// Helper: build a HandlerContext with a ContentIndex containing given tokens
    /// mapped to given files. Builds trigram index automatically.
    fn make_substring_ctx(
        tokens_to_files: Vec<(&str, u32, Vec<u32>)>,
        files: Vec<&str>,
    ) -> HandlerContext {
        let mut index_map: HashMap<String, Vec<Posting>> = HashMap::new();
        for (token, file_id, lines) in &tokens_to_files {
            index_map.entry(token.to_string()).or_default().push(Posting {
                file_id: *file_id,
                lines: lines.clone(),
            });
        }

        let file_token_counts: Vec<u32> = {
            let mut counts = vec![0u32; files.len()];
            for (_, file_id, lines) in &tokens_to_files {
                if (*file_id as usize) < counts.len() {
                    counts[*file_id as usize] += lines.len() as u32;
                }
            }
            counts
        };

        let total_tokens: u64 = file_token_counts.iter().map(|&c| c as u64).sum();

        let trigram = build_trigram_index(&index_map);

        let content_index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: files.iter().map(|s| s.to_string()).collect(),
            index: index_map,
            total_tokens,
            extensions: vec!["cs".to_string()],
            file_token_counts,
            trigram,
            trigram_dirty: false,
            forward: None,
            path_to_id: None,
        };

        HandlerContext {
            index: Arc::new(RwLock::new(content_index)),
            def_index: None,
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),

        }
    }

    #[test]
    fn test_substring_search_finds_partial_match() {
        // Index with token "databaseconnectionfactory"
        let ctx = make_substring_ctx(
            vec![("databaseconnectionfactory", 0, vec![10])],
            vec!["C:\\test\\Activity.cs"],
        );
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "databaseconn",
            "substring": true
        }));
        assert!(!result.is_error, "Expected success, got error: {:?}", result.content);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["totalFiles"], 1);
        let matched_tokens = output["summary"]["matchedTokens"].as_array().unwrap();
        assert!(matched_tokens.iter().any(|t| t.as_str() == Some("databaseconnectionfactory")));
    }

    #[test]
    fn test_substring_search_no_match() {
        let ctx = make_substring_ctx(
            vec![("httpclient", 0, vec![5])],
            vec!["C:\\test\\Program.cs"],
        );
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "xyznonexistent",
            "substring": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["totalFiles"], 0);
    }

    #[test]
    fn test_substring_search_full_token_match() {
        let ctx = make_substring_ctx(
            vec![("httpclient", 0, vec![5, 12])],
            vec!["C:\\test\\Program.cs"],
        );
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "httpclient",
            "substring": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["totalFiles"], 1);
        assert_eq!(output["files"][0]["occurrences"], 2);
    }

    #[test]
    fn test_substring_search_case_insensitive() {
        // Token is already lowercase in the index
        let ctx = make_substring_ctx(
            vec![("httpclient", 0, vec![5])],
            vec!["C:\\test\\Program.cs"],
        );
        // Query with mixed case â€” should be lowercased before matching
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "HttpCli",
            "substring": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["totalFiles"], 1);
    }

    #[test]
    fn test_substring_search_short_query_warning() {
        let ctx = make_substring_ctx(
            vec![("ab_something", 0, vec![1])],
            vec!["C:\\test\\File.cs"],
        );
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "ab",
            "substring": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        // Short query should produce a warning
        assert!(output["summary"]["warning"].is_string(),
            "Expected warning for short query, got: {:?}", output["summary"]);
    }

    #[test]
    fn test_substring_search_mutually_exclusive_with_regex() {
        let ctx = make_substring_ctx(
            vec![("httpclient", 0, vec![5])],
            vec!["C:\\test\\Program.cs"],
        );
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "http",
            "substring": true,
            "regex": true
        }));
        assert!(result.is_error);
        assert!(result.content[0].text.contains("mutually exclusive"));
    }

    #[test]
    fn test_substring_search_mutually_exclusive_with_phrase() {
        let ctx = make_substring_ctx(
            vec![("httpclient", 0, vec![5])],
            vec!["C:\\test\\Program.cs"],
        );
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "http",
            "substring": true,
            "phrase": true
        }));
        assert!(result.is_error);
        assert!(result.content[0].text.contains("mutually exclusive"));
    }

    #[test]
    fn test_substring_search_multi_term_or() {
        let ctx = make_substring_ctx(
            vec![
                ("httpclient", 0, vec![5]),
                ("grpchandler", 1, vec![10]),
            ],
            vec!["C:\\test\\Http.cs", "C:\\test\\Grpc.cs"],
        );
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "httpcli,grpchan",
            "substring": true,
            "mode": "or"
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["totalFiles"], 2);
    }

    #[test]
    fn test_substring_search_multi_term_and() {
        // Both tokens in the same file (file 0)
        let ctx = make_substring_ctx(
            vec![
                ("httpclient", 0, vec![5]),
                ("grpchandler", 0, vec![10]),
                ("grpchandler", 1, vec![20]),  // also in file 1 but without httpclient
            ],
            vec!["C:\\test\\Both.cs", "C:\\test\\GrpcOnly.cs"],
        );
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "httpcli,grpchan",
            "substring": true,
            "mode": "and"
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        // Only file 0 contains both terms
        assert_eq!(output["summary"]["totalFiles"], 1);
        assert_eq!(output["files"][0]["path"], "C:\\test\\Both.cs");
    }

    #[test]
    fn test_substring_search_count_only() {
        let ctx = make_substring_ctx(
            vec![
                ("httpclient", 0, vec![5, 12]),
                ("httphandler", 1, vec![3]),
            ],
            vec!["C:\\test\\Client.cs", "C:\\test\\Handler.cs"],
        );
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "http",
            "substring": true,
            "countOnly": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["totalFiles"], 2);
        assert_eq!(output["summary"]["totalOccurrences"], 3);
        // countOnly should NOT include "files" array
        assert!(output.get("files").is_none());
    }

    #[test]
    fn test_substring_search_trigram_dirty_triggers_rebuild() {
        // Create context with trigram_dirty=true to test lazy rebuild
        let mut index_map: HashMap<String, Vec<Posting>> = HashMap::new();
        index_map.insert("httpclient".to_string(), vec![Posting { file_id: 0, lines: vec![5] }]);

        let content_index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec!["C:\\test\\Program.cs".to_string()],
            index: index_map,
            total_tokens: 1,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![1],
            trigram: TrigramIndex::default(), // empty trigram
            trigram_dirty: true,              // needs rebuild
            forward: None,
            path_to_id: None,
        };

        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(content_index)),
            def_index: None,
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),

        };

        // Should trigger rebuild and still find the token
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "httpcli",
            "substring": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["totalFiles"], 1);

        // Verify trigram_dirty is now false
        let idx = ctx.index.read().unwrap();
        assert!(!idx.trigram_dirty, "trigram_dirty should be false after rebuild");
        assert!(!idx.trigram.tokens.is_empty(), "trigram tokens should be populated after rebuild");
    }

    // â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
    // E2E tests: build real index from files on disk â†’ query via dispatch_tool
    // â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

    /// Helper: create a temp dir with source files containing compound identifiers,
    /// build a real content index from it, and return a HandlerContext.
    fn make_e2e_substring_ctx() -> (HandlerContext, std::path::PathBuf) {
        use std::io::Write;

        static E2E_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = E2E_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tmp_dir = std::env::temp_dir().join(format!("search_e2e_{}_{}", std::process::id(), id));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        // File 1: Service.cs with compound identifiers
        {
            let mut f = std::fs::File::create(tmp_dir.join("Service.cs")).unwrap();
            writeln!(f, "using System;").unwrap();
            writeln!(f, "namespace MyApp {{").unwrap();
            writeln!(f, "    public class DatabaseConnectionFactory {{").unwrap();
            writeln!(f, "        private HttpClientHandler _handler;").unwrap();
            writeln!(f, "        public void Execute() {{").unwrap();
            writeln!(f, "            var provider = new GrpcServiceProvider();").unwrap();
            writeln!(f, "            _handler.Send();").unwrap();
            writeln!(f, "        }}").unwrap();
            writeln!(f, "    }}").unwrap();
            writeln!(f, "}}").unwrap();
        }

        // File 2: Controller.cs with different identifiers
        {
            let mut f = std::fs::File::create(tmp_dir.join("Controller.cs")).unwrap();
            writeln!(f, "using System;").unwrap();
            writeln!(f, "namespace MyApp {{").unwrap();
            writeln!(f, "    public class UserController {{").unwrap();
            writeln!(f, "        private readonly HttpClientHandler _client;").unwrap();
            writeln!(f, "        public async Task<IActionResult> GetAsync() {{").unwrap();
            writeln!(f, "            return Ok();").unwrap();
            writeln!(f, "        }}").unwrap();
            writeln!(f, "    }}").unwrap();
            writeln!(f, "}}").unwrap();
        }

        // File 3: Util.cs with unique token
        {
            let mut f = std::fs::File::create(tmp_dir.join("Util.cs")).unwrap();
            writeln!(f, "public static class CacheManagerHelper {{").unwrap();
            writeln!(f, "    public static void ClearAll() {{ }}").unwrap();
            writeln!(f, "}}").unwrap();
        }

        // Build a real content index from these files
        let content_index = crate::build_content_index(&crate::ContentIndexArgs {
            dir: tmp_dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: false,
            threads: 1,
            min_token_len: 2,
        });

        let ctx = HandlerContext {
            index: Arc::new(RwLock::new(content_index)),
            def_index: None,
            server_dir: tmp_dir.to_string_lossy().to_string(),
            server_ext: "cs".to_string(),

        };

        (ctx, tmp_dir)
    }

    // â”€â”€â”€ E2E Test 1: Full pipeline â€” build index â†’ substring search â”€â”€

    #[test]
    fn e2e_substring_search_full_pipeline() {
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        // Search for "databaseconn" â€” should match "databaseconnectionfactory"
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "databaseconn",
            "substring": true
        }));
        assert!(!result.is_error, "search should succeed");
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["searchMode"], "substring-or");
        assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 1,
            "Should find at least 1 file containing 'databaseconnectionfactory'");

        // Matched tokens should include "databaseconnectionfactory"
        let matched_tokens = output["summary"]["matchedTokens"].as_array().unwrap();
        assert!(matched_tokens.iter().any(|t| t.as_str().unwrap() == "databaseconnectionfactory"),
            "matchedTokens should include 'databaseconnectionfactory', got: {:?}", matched_tokens);

        // Verify the returned file path contains "Service.cs"
        let files = output["files"].as_array().unwrap();
        assert!(!files.is_empty());
        assert!(files.iter().any(|f| f["path"].as_str().unwrap().contains("Service.cs")),
            "Should find Service.cs");

        // Search for "httpclient" â€” should find in both Service.cs and Controller.cs
        let result2 = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "httpclient",
            "substring": true
        }));
        assert!(!result2.is_error);
        let output2: Value = serde_json::from_str(&result2.content[0].text).unwrap();
        assert!(output2["summary"]["totalFiles"].as_u64().unwrap() >= 2,
            "Should find at least 2 files containing 'httpclienthandler'");

        // Search for something that doesn't exist
        let result3 = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "nonexistentxyz",
            "substring": true
        }));
        assert!(!result3.is_error);
        let output3: Value = serde_json::from_str(&result3.content[0].text).unwrap();
        assert_eq!(output3["summary"]["totalFiles"], 0);

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 2: Substring search with showLines â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn e2e_substring_search_with_show_lines() {
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "grpcservice",
            "substring": true,
            "showLines": true,
            "contextLines": 1
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 1);

        // Should have lineContent in the file result
        let files = output["files"].as_array().unwrap();
        assert!(!files.is_empty(), "Should find at least one file");
        let first_file = &files[0];
        let line_content = first_file["lineContent"].as_array();
        assert!(line_content.is_some(), "Should have lineContent when showLines=true");
        let groups = line_content.unwrap();
        assert!(!groups.is_empty(), "lineContent should have at least one group");

        // Each group should have startLine, lines, matchIndices
        let group = &groups[0];
        assert!(group["startLine"].is_number(), "group should have startLine");
        assert!(group["lines"].is_array(), "group should have lines array");

        // The lines array should contain the actual source line
        let lines_arr = group["lines"].as_array().unwrap();
        let all_text: String = lines_arr.iter().map(|l| l.as_str().unwrap_or("")).collect::<Vec<_>>().join(" ");
        assert!(all_text.to_lowercase().contains("grpcserviceprovider"),
            "Line content should contain 'GrpcServiceProvider', got: {}", all_text);

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 3: Reindex rebuilds trigram â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn e2e_reindex_rebuilds_trigram() {
        use std::io::Write;
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        // First query â€” should find "cachemanagerhelper" via substring "cachemanager"
        let result1 = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "cachemanager",
            "substring": true
        }));
        assert!(!result1.is_error);
        let output1: Value = serde_json::from_str(&result1.content[0].text).unwrap();
        assert!(output1["summary"]["totalFiles"].as_u64().unwrap() >= 1,
            "Should find 'cachemanagerhelper' before reindex");

        // Modify files: remove Util.cs, add NewFile.cs with new identifier
        std::fs::remove_file(tmp_dir.join("Util.cs")).unwrap();
        {
            let mut f = std::fs::File::create(tmp_dir.join("NewFile.cs")).unwrap();
            writeln!(f, "public class DatabaseConnectionPoolManager {{}}").unwrap();
        }

        // Reindex
        let reindex_result = dispatch_tool(&ctx, "search_reindex", &json!({}));
        assert!(!reindex_result.is_error, "Reindex should succeed");

        // After reindex, "cachemanager" should no longer be found
        let result2 = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "cachemanager",
            "substring": true
        }));
        assert!(!result2.is_error);
        let output2: Value = serde_json::from_str(&result2.content[0].text).unwrap();
        assert_eq!(output2["summary"]["totalFiles"], 0,
            "Should NOT find 'cachemanagerhelper' after Util.cs was deleted and reindex");

        // After reindex, "connectionpool" should now be found
        let result3 = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "connectionpool",
            "substring": true
        }));
        assert!(!result3.is_error);
        let output3: Value = serde_json::from_str(&result3.content[0].text).unwrap();
        assert!(output3["summary"]["totalFiles"].as_u64().unwrap() >= 1,
            "Should find 'databaseconnectionpoolmanager' after reindex");

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 4: File change â†’ trigram dirty â†’ lazy rebuild â”€â”€â”€â”€â”€â”€

    #[test]
    fn e2e_watcher_trigram_dirty_lazy_rebuild() {
        use std::io::Write;
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        // Verify initial search works
        let result1 = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "databaseconn",
            "substring": true
        }));
        assert!(!result1.is_error);
        let output1: Value = serde_json::from_str(&result1.content[0].text).unwrap();
        assert!(output1["summary"]["totalFiles"].as_u64().unwrap() >= 1);

        // Simulate what the watcher does: update inverted index + set trigram_dirty
        {
            let mut idx = ctx.index.write().unwrap();

            // Add a new file with a new token to the inverted index
            let new_file_id = idx.files.len() as u32;
            let new_path = tmp_dir.join("Dynamic.cs");
            {
                let mut f = std::fs::File::create(&new_path).unwrap();
                writeln!(f, "public class AsyncBlobStorageProcessor {{}}").unwrap();
            }
            idx.files.push(clean_path(&new_path.to_string_lossy()));
            idx.file_token_counts.push(1);

            // Add token to inverted index
            idx.index.entry("asyncblobstorageprocessor".to_string())
                .or_default()
                .push(Posting { file_id: new_file_id, lines: vec![1] });
            idx.total_tokens += 1;

            // Mark trigram as dirty (what the watcher does)
            idx.trigram_dirty = true;
        }

        // Now search for the new token via substring â€” should trigger lazy trigram rebuild
        let result2 = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "blobstorage",
            "substring": true
        }));
        assert!(!result2.is_error);
        let output2: Value = serde_json::from_str(&result2.content[0].text).unwrap();
        assert!(output2["summary"]["totalFiles"].as_u64().unwrap() >= 1,
            "Should find 'asyncblobstorageprocessor' after lazy trigram rebuild");

        // Verify trigram is no longer dirty
        let idx = ctx.index.read().unwrap();
        assert!(!idx.trigram_dirty, "trigram_dirty should be false after lazy rebuild");

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 5: Index serialization roundtrip with trigram â”€â”€â”€â”€â”€â”€

    #[test]
    fn e2e_index_serialization_roundtrip_with_trigram() {
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        // Get the original index
        let original_index = ctx.index.read().unwrap();
        let original_file_count = original_index.files.len();
        let original_token_count = original_index.index.len();
        let original_trigram_count = original_index.trigram.trigram_map.len();
        let original_trigram_token_count = original_index.trigram.tokens.len();
        assert!(original_trigram_count > 0, "Trigram index should be populated");
        assert!(original_trigram_token_count > 0, "Trigram tokens should be populated");

        // Save to disk
        crate::save_content_index(&original_index).unwrap();
        let root = original_index.root.clone();
        let exts = original_index.extensions.join(",");
        drop(original_index);

        // Load from disk
        let loaded_index = crate::load_content_index(&root, &exts)
            .expect("Should load saved content index");

        // Verify structural equality
        assert_eq!(loaded_index.files.len(), original_file_count);
        assert_eq!(loaded_index.index.len(), original_token_count);
        assert_eq!(loaded_index.trigram.trigram_map.len(), original_trigram_count);
        assert_eq!(loaded_index.trigram.tokens.len(), original_trigram_token_count);
        assert!(!loaded_index.trigram_dirty);

        // Build a new context with the loaded index and query it
        let loaded_ctx = HandlerContext {
            index: Arc::new(RwLock::new(loaded_index)),
            def_index: None,
            server_dir: root,
            server_ext: "cs".to_string(),

        };

        // Same substring search should produce same results
        let result = dispatch_tool(&loaded_ctx, "search_grep", &json!({
            "terms": "databaseconn",
            "substring": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 1,
            "Loaded index should find same results as original");

        let matched = output["summary"]["matchedTokens"].as_array().unwrap();
        assert!(matched.iter().any(|t| t.as_str().unwrap() == "databaseconnectionfactory"),
            "Loaded index should find 'databaseconnectionfactory'");

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 6: Substring with multi-term AND mode â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn e2e_substring_search_multi_term_and() {
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        // "httpclient" exists in Service.cs and Controller.cs
        // "grpcservice" exists only in Service.cs
        // AND mode should only return Service.cs
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "httpclient,grpcservice",
            "substring": true,
            "mode": "and"
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["searchMode"], "substring-and");
        assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 1,
            "AND search should find at least 1 file");

        let files = output["files"].as_array().unwrap();
        // All returned files should contain BOTH terms
        for file in files {
            let path = file["path"].as_str().unwrap();
            assert!(path.contains("Service.cs"),
                "AND mode should only return Service.cs (has both), got: {}", path);
        }

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 7: Substring search count-only mode â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn e2e_substring_search_count_only() {
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "httpclient",
            "substring": true,
            "countOnly": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

        // count_only should have summary but no files array
        assert!(output.get("files").is_none(), "countOnly should not include files array");
        assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 2);
        assert!(output["summary"]["totalOccurrences"].as_u64().unwrap() >= 2);

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 8: Substring search with exclude filters â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn e2e_substring_search_with_excludes() {
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        // Search for "httpclient" but exclude Controller
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "httpclient",
            "substring": true,
            "exclude": ["Controller"]
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

        let files = output["files"].as_array().unwrap();
        for file in files {
            let path = file["path"].as_str().unwrap();
            assert!(!path.contains("Controller"),
                "Excluded files should not appear, got: {}", path);
        }

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 9: Substring search with maxResults â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn e2e_substring_search_max_results() {
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        // Search for a broad term that matches multiple files, limit results
        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "public",
            "substring": true,
            "maxResults": 1
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

        let files = output["files"].as_array().unwrap();
        assert!(files.len() <= 1, "maxResults=1 should return at most 1 file, got: {}", files.len());

        // But totalFiles in summary should still report the true count
        assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 1);

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 10: Short substring (<4 chars) produces warning â”€â”€â”€

    #[test]
    fn e2e_substring_search_short_query_warning() {
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "ok",
            "substring": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

        // Short query should produce a warning
        assert!(output["summary"]["warning"].is_string(),
            "Short query should produce a warning, got summary: {}", output["summary"]);
        let warning = output["summary"]["warning"].as_str().unwrap();
        assert!(warning.contains("Short substring"), "Warning should mention short query");

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 11: Substring mutually exclusive with regex â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn e2e_substring_mutually_exclusive_with_regex() {
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "test",
            "substring": true,
            "regex": true
        }));
        assert!(result.is_error, "substring + regex should be an error");
        assert!(result.content[0].text.contains("mutually exclusive"));

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 12: Substring mutually exclusive with phrase â”€â”€â”€â”€â”€â”€

    #[test]
    fn e2e_substring_mutually_exclusive_with_phrase() {
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "test",
            "substring": true,
            "phrase": true
        }));
        assert!(result.is_error, "substring + phrase should be an error");
        assert!(result.content[0].text.contains("mutually exclusive"));

        cleanup_tmp(&tmp_dir);
    }

    // â”€â”€â”€ E2E Test 13: Verify TF-IDF scoring in substring results â”€â”€â”€â”€

    #[test]
    fn e2e_substring_search_has_scores() {
        let (ctx, tmp_dir) = make_e2e_substring_ctx();

        let result = dispatch_tool(&ctx, "search_grep", &json!({
            "terms": "httpclient",
            "substring": true
        }));
        assert!(!result.is_error);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

        let files = output["files"].as_array().unwrap();
        assert!(!files.is_empty());

        for file in files {
            assert!(file["score"].is_number(), "Each file should have a TF-IDF score");
            assert!(file["occurrences"].is_number(), "Each file should have occurrences");
            assert!(file["lines"].is_array(), "Each file should have lines array");
        }

        // Results should be sorted by score descending
        if files.len() >= 2 {
            let score0 = files[0]["score"].as_f64().unwrap();
            let score1 = files[1]["score"].as_f64().unwrap();
            assert!(score0 >= score1, "Results should be sorted by score descending");
        }

        cleanup_tmp(&tmp_dir);
    }
}
