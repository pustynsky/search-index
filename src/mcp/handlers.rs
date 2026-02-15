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
use crate::definitions::{DefinitionEntry, DefinitionIndex, DefinitionKind};

/// Return all tool definitions for tools/list
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "search_grep".to_string(),
            description: "Search file contents using an inverted index with TF-IDF ranking. Supports exact tokens, multi-term OR/AND, regex, phrase search, and exclusion filters. Results ranked by relevance. Index stays in memory for instant subsequent queries (~0.001s). IMPORTANT: When searching for all usages of a class/interface, use multi-term OR search to find ALL naming variants in ONE query. Example: to find all usages of MyClass, search for 'MyClass,IMyClass,MyClassFactory' with mode='or'. This is much faster than making separate queries for each variant. Comma-separated terms with mode='or' finds files containing ANY of the terms; mode='and' finds files containing ALL terms.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "terms": {
                        "type": "string",
                        "description": "Search terms. Comma-separated for multi-term search. Single token: 'HttpClient'. Multi-term OR/AND: 'HttpClient,ILogger,Task' (finds files with ANY term when mode='or', or ALL terms when mode='and'). Always use comma-separated multi-term OR search when looking for all usages of a class — include the class name, its interface, and related types in one query. Phrase (use with phrase=true): 'new HttpClient'. Regex (use with regex=true): 'I.*Cache'"
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
                        "description": "Include actual source code line content from matching files in the results. When true, each result file includes a 'lineContent' array with line numbers and text. Use this to READ the actual code at match locations without needing a separate read_file call. Combine with contextLines to see surrounding code. (default: false)"
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
                    }
                },
                "required": ["terms"]
            }),
        },
        ToolDefinition {
            name: "search_find".to_string(),
            description: "Search for files by name using live filesystem walk. No index needed. ⚠️ WARNING: This performs a live filesystem walk and may be slow for large directories (10-30s). For instant results, use search_fast with a pre-built file name index.".to_string(),
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
            name: "search_definitions".to_string(),
            description: "Search C# and SQL code definitions — classes, interfaces, methods, properties, enums, stored procedures, tables. Uses pre-built tree-sitter AST index for instant results (~0.001s). Requires server started with --definitions flag. Supports 'containsLine' to find which method/class contains a given line number (no more manual read_file!).".to_string(),
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
                        "description": "Find the definition(s) that contain this line number. Returns the innermost method/property and its parent class. Must be used with 'file' parameter. Example: file='UserService.cs', containsLine=42 → returns GetUserAsync (lines 35-50), parent: UserService"
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
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "search_callers".to_string(),
            description: "Find all callers of a method and build a call tree (up or down). Combines grep index (to find where a method name appears) with AST definition index (to determine which method/class contains each call site). Returns a hierarchical call tree. This is the most powerful tool for tracing call chains — replaces 7+ sequential search_grep + read_file calls with a single request. Requires server started with --definitions flag.".to_string(),
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

/// Context for tool handlers — shared state
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
        "search_definitions" => handle_search_definitions(ctx, arguments),
        "search_callers" => handle_search_callers(ctx, arguments),
        _ => ToolCallResult::error(format!("Unknown tool: {}", tool_name)),
    }
}

// ─── search_grep handler ─────────────────────────────────────────────

fn handle_search_grep(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
    let terms_str = match args.get("terms").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return ToolCallResult::error("Missing required parameter: terms".to_string()),
    };

    // Check dir parameter — must match server dir or be absent
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

    let index = match ctx.index.read() {
        Ok(idx) => idx,
        Err(e) => return ToolCallResult::error(format!("Failed to acquire index lock: {}", e)),
    };

    // ─── Phrase search mode ─────────────────────────────────
    if use_phrase {
        return handle_phrase_search(
            &index, &terms_str, &ext_filter, &exclude_dir, &exclude,
            show_lines, context_lines, max_results, count_only, search_start,
        );
    }

    // ─── Normal token search ────────────────────────────────
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
                let lines_vec: Vec<&str> = content.lines().collect();
                let total_lines = lines_vec.len();
                let mut line_content = Vec::new();

                let mut lines_to_show: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
                let mut match_lines_set: std::collections::HashSet<usize> = std::collections::HashSet::new();

                for &ln in &r.lines {
                    let idx = (ln as usize).saturating_sub(1);
                    if idx < total_lines {
                        match_lines_set.insert(idx);
                        let s = idx.saturating_sub(context_lines);
                        let e = (idx + context_lines).min(total_lines - 1);
                        for i in s..=e { lines_to_show.insert(i); }
                    }
                }

                for &idx in &lines_to_show {
                    line_content.push(json!({
                        "line": idx + 1,
                        "text": lines_vec[idx],
                        "isMatch": match_lines_set.contains(&idx),
                    }));
                }

                file_obj["lineContent"] = json!(line_content);
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
                let lines_vec: Vec<&str> = content.lines().collect();
                let total_lines = lines_vec.len();
                let mut line_content = Vec::new();

                let mut lines_to_show: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
                let mut match_lines_set: std::collections::HashSet<usize> = std::collections::HashSet::new();

                for &ln in &r.lines {
                    let idx = (ln as usize).saturating_sub(1);
                    if idx < total_lines {
                        match_lines_set.insert(idx);
                        let s = idx.saturating_sub(context_lines);
                        let e = (idx + context_lines).min(total_lines - 1);
                        for i in s..=e { lines_to_show.insert(i); }
                    }
                }

                for &idx in &lines_to_show {
                    line_content.push(json!({
                        "line": idx + 1,
                        "text": lines_vec[idx],
                        "isMatch": match_lines_set.contains(&idx),
                    }));
                }

                file_obj["lineContent"] = json!(line_content);
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

// ─── search_find handler ─────────────────────────────────────────────

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

// ─── search_fast handler ─────────────────────────────────────────────

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

// ─── search_info handler ─────────────────────────────────────────────

fn handle_search_info() -> ToolCallResult {
    let info = cmd_info_json();
    ToolCallResult::success(serde_json::to_string(&info).unwrap())
}

// ─── search_reindex handler ──────────────────────────────────────────

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

// ─── search_definitions handler ──────────────────────────────────────

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

    // ─── containsLine: find containing method/class by line number ───
    if let Some(line_num) = contains_line {
        if file_filter.is_none() {
            return ToolCallResult::error(
                "containsLine requires 'file' parameter to identify the file.".to_string()
            );
        }
        let file_substr = file_filter.unwrap().to_lowercase();

        // Find matching file(s)
        let mut containing_defs: Vec<Value> = Vec::new();
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
                    containing_defs.push(obj);
                }
            }
        }

        let search_elapsed = search_start.elapsed();
        let output = json!({
            "containingDefinitions": containing_defs,
            "query": {
                "file": file_filter.unwrap(),
                "line": line_num,
            },
            "summary": {
                "totalResults": containing_defs.len(),
                "searchTimeMs": search_elapsed.as_secs_f64() * 1000.0,
            }
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

        obj
    }).collect();

    let output = json!({
        "definitions": defs_json,
        "summary": {
            "totalResults": total_results,
            "returned": defs_json.len(),
            "searchTimeMs": search_elapsed.as_secs_f64() * 1000.0,
            "indexFiles": index.files.len(),
            "totalDefinitions": index.definitions.len(),
        }
    });

    ToolCallResult::success(serde_json::to_string(&output).unwrap())
}

// ─── search_callers handler ──────────────────────────────────────────

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
            max_depth,
            0,
            &content_index,
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
        let output = json!({
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
/// `parent_class` is used to disambiguate common method names — when recursing,
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

/// Check if a token is a C# keyword, common type, or noise that should not be
/// treated as a method call when building callee trees.
fn is_csharp_noise_token(token: &str) -> bool {
    matches!(token,
        // C# keywords
        "return" | "async" | "await" | "var" | "null" | "true" | "false"
        | "if" | "else" | "for" | "foreach" | "while" | "do" | "switch"
        | "case" | "break" | "continue" | "throw" | "try" | "catch" | "finally"
        | "using" | "namespace" | "class" | "struct" | "interface" | "enum"
        | "public" | "private" | "protected" | "internal" | "static" | "readonly"
        | "const" | "sealed" | "abstract" | "virtual" | "override" | "new"
        | "this" | "base" | "typeof" | "sizeof" | "nameof" | "default"
        | "void" | "object" | "string" | "bool" | "int" | "long" | "double"
        | "float" | "decimal" | "byte" | "short" | "uint" | "ulong" | "char"
        | "where" | "select" | "from" | "orderby" | "group" | "into" | "join"
        | "let" | "ascending" | "descending" | "equals" | "value" | "get" | "set"
        | "add" | "remove" | "partial" | "yield" | "lock" | "fixed" | "checked"
        | "unchecked" | "unsafe" | "volatile" | "extern" | "ref" | "out" | "in"
        | "is" | "as" | "params" | "delegate" | "event" | "implicit" | "explicit"
        | "operator" | "stackalloc" | "when" | "with" | "record" | "init"
        // Common .NET types and patterns (lowercased tokens)
        | "task" | "list" | "dictionary" | "hashset" | "ienumerable"
        | "ilist" | "icollection" | "ireadonlylist" | "ireadonlycollection"
        | "cancellationtoken" | "exception" | "argumentexception"
        | "argumentnullexception" | "invalidoperationexception"
        | "notimplementedexception" | "notsupportedexception"
        | "keyvaluepair" | "nullable" | "func" | "action" | "predicate"
        | "tuple" | "valuetuple" | "guid" | "datetime" | "timespan" | "uri"
        | "type" | "array" | "span" | "memory" | "readonlyspan" | "readonlymemory"
        // Common test/assertion patterns
        | "assert" | "verify" | "mock" | "setup" | "returns" | "throws"
        | "should" | "expect" | "actual" | "expected" | "result"
        // Common noise in method bodies
        | "empty" | "count" | "length" | "first" | "last" | "single"
        | "any" | "all" | "contains" | "tostring" | "toarray" | "tolist"
        | "tolower" | "toupper" | "trim" | "split" | "format" | "concat"
        | "gethashcode" | "gettype" | "dispose" | "close"
        | "configureawait" | "completedtask" | "fromresult" | "whenall"
    )
}

/// Build a callee tree (direction = "down"): find what methods are called by this method.
fn build_callee_tree(
    method_name: &str,
    max_depth: usize,
    current_depth: usize,
    content_index: &ContentIndex,
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
    if !visited.insert(method_lower.clone()) {
        return Vec::new();
    }

    let method_defs: Vec<&DefinitionEntry> = def_idx.name_index
        .get(&method_lower)
        .map(|indices| {
            indices.iter()
                .filter_map(|&di| def_idx.definitions.get(di as usize))
                .filter(|d| d.kind == DefinitionKind::Method || d.kind == DefinitionKind::Constructor)
                .collect()
        })
        .unwrap_or_default();

    if method_defs.is_empty() {
        return Vec::new();
    }

    let mut callees: Vec<Value> = Vec::new();
    let mut seen_callees: std::collections::HashSet<String> = std::collections::HashSet::new();

    for method_def in &method_defs {
        if callees.len() >= limits.max_callers_per_level { break; }
        if node_count.load(std::sync::atomic::Ordering::Relaxed) >= limits.max_total_nodes { break; }

        let file_path = match def_idx.files.get(method_def.file_id as usize) {
            Some(p) => p,
            None => continue,
        };

        // Find the content_index file_id for this file (different numbering from def_idx)
        let content_file_id: Option<u32> = content_index.files.iter()
            .position(|f| f == file_path)
            .map(|i| i as u32);

        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let start = (method_def.line_start as usize).saturating_sub(1);
        let end = (method_def.line_end as usize).min(lines.len());

        for line_idx in start..end {
            if callees.len() >= limits.max_callers_per_level { break; }
            if node_count.load(std::sync::atomic::Ordering::Relaxed) >= limits.max_total_nodes { break; }

            let line = lines[line_idx];
            let line_tokens = tokenize(line, 2);
            for token in &line_tokens {
                if token == &method_lower { continue; }

                // Skip C# keywords and common types that are never method calls
                if is_csharp_noise_token(token) { continue; }

                if let Some(name_indices) = def_idx.name_index.get(token.as_str()) {
                    // Skip methods with many definitions (Trace, Log, etc.)
                    // These are ubiquitous and produce noise. Only include
                    // methods that have a small number of definitions (unique enough).
                    if name_indices.len() > 5 {
                        continue;
                    }

                    for &di in name_indices {
                        if let Some(callee_def) = def_idx.definitions.get(di as usize) {
                            if callee_def.kind != DefinitionKind::Method && callee_def.kind != DefinitionKind::Constructor {
                                continue;
                            }

                            // Only include callees whose parent class is
                            // referenced in the same file (via the content index).
                            // Use content_file_id (not method_def.file_id which is def_idx numbering)
                            if let (Some(parent), Some(cfid)) = (&callee_def.parent, content_file_id) {
                                let parent_lower = parent.to_lowercase();
                                let has_ref = content_index.index.get(&parent_lower)
                                    .is_some_and(|p| p.iter().any(|pp| pp.file_id == cfid));
                                if !has_ref {
                                    let iface = format!("i{}", parent_lower);
                                    let has_iface = content_index.index.get(&iface)
                                        .is_some_and(|p| p.iter().any(|pp| pp.file_id == cfid));
                                    if !has_iface { continue; }
                                }
                            }

                            let callee_file = def_idx.files.get(callee_def.file_id as usize)
                                .map(|s| s.as_str()).unwrap_or("");

                            let matches_ext = Path::new(callee_file)
                                .extension()
                                .and_then(|e| e.to_str())
                                .is_some_and(|e| e.eq_ignore_ascii_case(ext_filter));
                            if !matches_ext { continue; }

                            let callee_key = format!("{}.{}",
                                callee_def.parent.as_deref().unwrap_or("?"),
                                &callee_def.name
                            );

                            if seen_callees.contains(&callee_key) { continue; }
                            seen_callees.insert(callee_key.clone());

                            node_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                            let sub_callees = build_callee_tree(
                                &callee_def.name,
                                max_depth,
                                current_depth + 1,
                                content_index,
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
                            });
                            if let Some(ref parent) = callee_def.parent {
                                node["class"] = json!(parent);
                            }
                            if let Some(fname) = Path::new(callee_file).file_name().and_then(|f| f.to_str()) {
                                node["file"] = json!(fname);
                            }
                            if !sub_callees.is_empty() {
                                node["callees"] = json!(sub_callees);
                            }
                            callees.push(node);
                        }
                    }
                }
            }
        }
    }

    callees
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Posting;

    #[test]
    fn test_tool_definitions_count() {
        let tools = tool_definitions();
        assert_eq!(tools.len(), 7);
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

    // ─── Helper: create a context with both content + definition indexes ───

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
        };

        HandlerContext {
            index: Arc::new(RwLock::new(content_index)),
            def_index: Some(Arc::new(RwLock::new(def_index))),
            server_dir: ".".to_string(),
            server_ext: "cs".to_string(),
        }
    }

    // ─── search_callers tests ───

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
            extensions: vec![], file_token_counts: vec![], forward: None, path_to_id: None,
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

    // ─── containsLine tests ───

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

    // ─── find_containing_method tests ───

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

    // ─── search_callers schema tests ───

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
    // ─── noise token filter tests ───

    #[test]
    fn test_csharp_noise_filter_keywords() {
        assert!(is_csharp_noise_token("return"));
        assert!(is_csharp_noise_token("async"));
        assert!(is_csharp_noise_token("await"));
        assert!(is_csharp_noise_token("var"));
        assert!(is_csharp_noise_token("null"));
        assert!(is_csharp_noise_token("true"));
        assert!(is_csharp_noise_token("false"));
        assert!(is_csharp_noise_token("if"));
        assert!(is_csharp_noise_token("public"));
        assert!(is_csharp_noise_token("private"));
        assert!(is_csharp_noise_token("static"));
    }

    #[test]
    fn test_csharp_noise_filter_types() {
        assert!(is_csharp_noise_token("task"));
        assert!(is_csharp_noise_token("cancellationtoken"));
        assert!(is_csharp_noise_token("string"));
        assert!(is_csharp_noise_token("list"));
        assert!(is_csharp_noise_token("exception"));
        assert!(is_csharp_noise_token("guid"));
    }

    #[test]
    fn test_csharp_noise_filter_test_patterns() {
        assert!(is_csharp_noise_token("assert"));
        assert!(is_csharp_noise_token("verify"));
        assert!(is_csharp_noise_token("mock"));
        assert!(is_csharp_noise_token("result"));
    }

    #[test]
    fn test_csharp_noise_filter_allows_real_methods() {
        assert!(!is_csharp_noise_token("executequeryasync"));
        assert!(!is_csharp_noise_token("runquerybatchasync"));
        assert!(!is_csharp_noise_token("processbatchqueryasync"));
        assert!(!is_csharp_noise_token("syncresourcesinternalasync"));
        assert!(!is_csharp_noise_token("queryservice"));
        assert!(!is_csharp_noise_token("httpclient"));
    }
}
