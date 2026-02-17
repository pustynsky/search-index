//! Shared utility functions for MCP tool handlers.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;

use serde_json::{json, Value};

use crate::mcp::protocol::ToolCallResult;
use crate::clean_path;

use super::HandlerContext;

// ─── Dir validation ─────────────────────────────────────────────────

/// Normalize path separators to forward slashes for cross-platform comparison.
pub(crate) fn normalize_path_sep(p: &str) -> String {
    p.replace('\\', "/")
}

/// Validate that `requested_dir` is the server dir or a subdirectory of it.
/// Returns `Ok(None)` if exact match (no filtering needed),
/// `Ok(Some(canonical_subdir))` if it's a proper subdirectory (use as filter),
/// or `Err(message)` if outside the server dir.
pub(crate) fn validate_search_dir(requested_dir: &str, server_dir: &str) -> Result<Option<String>, String> {
    let requested = std::fs::canonicalize(requested_dir)
        .map(|p| clean_path(&p.to_string_lossy()))
        .unwrap_or_else(|_| requested_dir.to_string());
    let server = std::fs::canonicalize(server_dir)
        .map(|p| clean_path(&p.to_string_lossy()))
        .unwrap_or_else(|_| server_dir.to_string());

    let req_norm = normalize_path_sep(&requested).to_lowercase();
    let srv_norm = normalize_path_sep(&server).to_lowercase();

    if req_norm == srv_norm {
        Ok(None)
    } else if req_norm.starts_with(&srv_norm) {
        // Verify it's a true subdirectory (next char must be '/')
        let next_char = req_norm.as_bytes().get(srv_norm.len());
        if next_char == Some(&b'/') {
            Ok(Some(requested))
        } else {
            Err(format!(
                "Server started with --dir {}. For other directories, start another server instance or use CLI.",
                server_dir
            ))
        }
    } else {
        Err(format!(
            "Server started with --dir {}. For other directories, start another server instance or use CLI.",
            server_dir
        ))
    }
}

/// Check if a file path is under the given directory prefix (case-insensitive, separator-normalized).
/// Ensures proper boundary check: `C:\Repos\Shared` won't match `C:\Repos\SharedExtra\file.cs`.
pub(crate) fn is_under_dir(file_path: &str, dir_prefix: &str) -> bool {
    let file_norm = normalize_path_sep(file_path).to_lowercase();
    let mut dir_norm = normalize_path_sep(dir_prefix).to_lowercase();
    // Ensure dir prefix ends with '/' for proper boundary matching
    if !dir_norm.ends_with('/') {
        dir_norm.push('/');
    }
    file_norm.starts_with(&dir_norm)
}

// ─── Set operations ─────────────────────────────────────────────────

/// Merge-intersect two sorted u32 slices. Returns sorted intersection.
pub(crate) fn sorted_intersect(a: &[u32], b: &[u32]) -> Vec<u32> {
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

// ─── Line content helpers ───────────────────────────────────────────

/// Build compact grouped lineContent for search_grep from raw file content.
/// Computes context windows around match lines, then groups consecutive lines
/// into `[{startLine, lines[], matchIndices[]}]`.
pub(crate) fn build_line_content_from_matches(
    content: &str,
    match_lines: &[u32],
    context_lines: usize,
) -> Value {
    let lines_vec: Vec<&str> = content.lines().collect();
    let total_lines = lines_vec.len();

    let mut lines_to_show = BTreeSet::new();
    let mut match_lines_set = HashSet::new();

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
pub(crate) fn build_grouped_line_content(
    lines_to_show: &BTreeSet<usize>,
    lines_vec: &[&str],
    match_lines_set: &HashSet<usize>,
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

// ─── Metrics injection ──────────────────────────────────────────────

/// Inject performance metrics into a successful tool response.
/// Parses the JSON text, adds searchTimeMs/responseBytes/estimatedTokens/indexFiles/indexTokens
/// to the "summary" object (if present), then re-serializes.
pub(crate) fn inject_metrics(result: ToolCallResult, ctx: &HandlerContext, start: Instant) -> ToolCallResult {
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    // Get the text from the first content item
    let text = match result.content.first() {
        Some(c) => &c.text,
        None => return result,
    };

    // Try to parse as JSON and inject metrics into "summary"
    if let Ok(mut output) = serde_json::from_str::<Value>(text) {
        if let Some(summary) = output.get_mut("summary") {
            summary["searchTimeMs"] = json!((elapsed_ms * 100.0).round() / 100.0);

            if let Ok(idx) = ctx.index.read() {
                summary["indexFiles"] = json!(idx.files.len());
                summary["indexTokens"] = json!(idx.index.len());
            }
        }

        // Measure response size after adding timing metrics
        let json_str = serde_json::to_string(&output).unwrap();
        let bytes = json_str.len();
        if let Some(summary) = output.get_mut("summary") {
            summary["responseBytes"] = json!(bytes);
            summary["estimatedTokens"] = json!(bytes / 4);
        }

        ToolCallResult::success(serde_json::to_string(&output).unwrap())
    } else {
        // Not valid JSON or no summary -- return as-is
        result
    }
}

// ─── Body injection helper ──────────────────────────────────────────

pub(crate) fn inject_body_into_obj(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sorted_intersect_empty_left() {
        assert_eq!(sorted_intersect(&[], &[1, 2, 3]), Vec::<u32>::new());
    }

    #[test]
    fn test_sorted_intersect_empty_right() {
        assert_eq!(sorted_intersect(&[1, 2, 3], &[]), Vec::<u32>::new());
    }

    #[test]
    fn test_sorted_intersect_both_empty() {
        assert_eq!(sorted_intersect(&[], &[]), Vec::<u32>::new());
    }

    #[test]
    fn test_sorted_intersect_disjoint() {
        assert_eq!(sorted_intersect(&[1, 3, 5], &[2, 4, 6]), Vec::<u32>::new());
    }

    #[test]
    fn test_normalize_path_sep() {
        assert_eq!(normalize_path_sep(r"C:\foo\bar"), "C:/foo/bar");
    }

    #[test]
    fn test_is_under_dir_basic() {
        assert!(is_under_dir("C:/Repos/MyProject/src/file.cs", "C:/Repos/MyProject"));
    }

    #[test]
    fn test_is_under_dir_case_insensitive() {
        assert!(is_under_dir("C:/repos/myproject/src/file.cs", "C:/Repos/MyProject"));
    }

    #[test]
    fn test_is_under_dir_not_prefix_of_different_dir() {
        assert!(!is_under_dir("C:/Repos/SharedExtra/file.cs", "C:/Repos/Shared"));
    }

    #[test]
    fn test_is_under_dir_exact_match() {
        assert!(!is_under_dir("C:/Repos/Shared", "C:/Repos/Shared"));
    }

    #[test]
    fn test_validate_search_dir_exact_match() {
        // We can't easily test this without real directories, but we can test the logic
        // with paths that don't exist (canonicalize will fail, falling back to raw string)
        let result = validate_search_dir("/nonexistent/dir", "/nonexistent/dir");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_validate_search_dir_outside_rejects() {
        let result = validate_search_dir("/other/dir", "/my/dir");
        assert!(result.is_err());
    }

    #[test]
    fn test_grouped_line_content_single_group() {
        let lines = vec!["line0", "line1", "line2", "line3", "line4"];
        let mut to_show = BTreeSet::new();
        to_show.insert(1);
        to_show.insert(2);
        to_show.insert(3);
        let mut match_set = HashSet::new();
        match_set.insert(2);

        let result = build_grouped_line_content(&to_show, &lines, &match_set);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["startLine"], 2);
        assert_eq!(groups[0]["lines"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_grouped_line_content_two_groups() {
        let lines = vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        let mut to_show = BTreeSet::new();
        to_show.insert(1);
        to_show.insert(2);
        to_show.insert(7);
        to_show.insert(8);
        let mut match_set = HashSet::new();
        match_set.insert(1);
        match_set.insert(8);

        let result = build_grouped_line_content(&to_show, &lines, &match_set);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn test_grouped_line_content_no_matches() {
        let lines = vec!["a", "b", "c"];
        let mut to_show = BTreeSet::new();
        to_show.insert(0);
        let match_set = HashSet::new();

        let result = build_grouped_line_content(&to_show, &lines, &match_set);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert!(groups[0].get("matchIndices").is_none());
    }

    #[test]
    fn test_grouped_line_content_empty() {
        let lines: Vec<&str> = vec![];
        let to_show = BTreeSet::new();
        let match_set = HashSet::new();

        let result = build_grouped_line_content(&to_show, &lines, &match_set);
        let groups = result.as_array().unwrap();
        assert!(groups.is_empty());
    }

    #[test]
    fn test_grouped_line_content_multiple_matches_in_group() {
        let lines = vec!["a", "b", "c", "d", "e"];
        let mut to_show = BTreeSet::new();
        for i in 0..5 { to_show.insert(i); }
        let mut match_set = HashSet::new();
        match_set.insert(1);
        match_set.insert(3);

        let result = build_grouped_line_content(&to_show, &lines, &match_set);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 1);
        let indices = groups[0]["matchIndices"].as_array().unwrap();
        assert_eq!(indices.len(), 2);
    }

    #[test]
    fn test_context_lines_calculation() {
        let content = (0..20).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
        let match_lines = vec![10u32]; // line 10 (1-based)
        let result = build_line_content_from_matches(&content, &match_lines, 2);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 1);
        // Should show lines 8-12 (5 lines: 2 before + match + 2 after)
        let lines = groups[0]["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn test_context_lines_at_file_boundaries() {
        let content = "line1\nline2\nline3";
        let match_lines = vec![1u32];
        let result = build_line_content_from_matches(&content, &match_lines, 5);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 1);
        let lines = groups[0]["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 3); // can't go before line 1
    }

    #[test]
    fn test_context_merges_overlapping_ranges() {
        let content = (0..20).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
        let match_lines = vec![5u32, 7u32]; // lines 5 and 7 with context 2 overlap
        let result = build_line_content_from_matches(&content, &match_lines, 2);
        let groups = result.as_array().unwrap();
        assert_eq!(groups.len(), 1); // should merge into single group
    }
}