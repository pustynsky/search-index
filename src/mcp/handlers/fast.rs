//! search_fast handler: pre-built file name index search.

use std::path::Path;
use std::time::Instant;

use serde_json::{json, Value};
use tracing::info;

use crate::mcp::protocol::ToolCallResult;

use super::HandlerContext;

pub(crate) fn handle_search_fast(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
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
    let index = match crate::load_index(&dir, &ctx.index_base) {
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
            let _ = crate::save_index(&new_index, &ctx.index_base);
            new_index
        }
    };

    // Split comma-separated patterns into multiple terms for OR matching
    let terms: Vec<String> = pattern
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let search_terms: Vec<String> = if ignore_case {
        terms.iter().map(|t| t.to_lowercase()).collect()
    } else {
        terms.clone()
    };

    let re_list: Option<Vec<regex::Regex>> = if use_regex {
        let mut regexes = Vec::with_capacity(terms.len());
        for t in &terms {
            let pat = if ignore_case { format!("(?i){}", t) } else { t.clone() };
            match regex::Regex::new(&pat) {
                Ok(r) => regexes.push(r),
                Err(e) => return ToolCallResult::error(format!("Invalid regex '{}': {}", t, e)),
            }
        }
        Some(regexes)
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

        let matched = if let Some(ref regexes) = re_list {
            regexes.iter().any(|re| re.is_match(&search_name))
        } else {
            search_terms.iter().any(|term| search_name.contains(term.as_str()))
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