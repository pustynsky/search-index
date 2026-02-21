//! search_grep handler: token search, substring search, phrase search.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use serde_json::{json, Value};

use crate::mcp::protocol::ToolCallResult;
use crate::{tokenize, ContentIndex};
use crate::index::build_trigram_index;
use search::generate_trigrams;

use super::utils::{
    build_line_content_from_matches, is_under_dir, sorted_intersect, validate_search_dir,
};
use super::HandlerContext;

pub(crate) struct FileScoreEntry {
    pub file_path: String,
    pub lines: Vec<u32>,
    pub tf_idf: f64,
    pub occurrences: usize,
    pub terms_matched: usize,
}

pub(crate) fn handle_search_grep(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
    let terms_str = match args.get("terms").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return ToolCallResult::error("Missing required parameter: terms".to_string()),
    };

    // Check dir parameter -- must match server dir or be a subdirectory
    let dir_filter: Option<String> = if let Some(dir) = args.get("dir").and_then(|v| v.as_str()) {
        match validate_search_dir(dir, &ctx.server_dir) {
            Ok(filter) => filter,
            Err(msg) => return ToolCallResult::error(msg),
        }
    } else {
        None
    };

    let ext_filter = args.get("ext").and_then(|v| v.as_str()).map(|s| s.to_string());
    let mode_and = args.get("mode").and_then(|v| v.as_str()) == Some("and");
    let use_regex = args.get("regex").and_then(|v| v.as_bool()).unwrap_or(false);
    let use_phrase = args.get("phrase").and_then(|v| v.as_bool()).unwrap_or(false);
    // Default to substring=true so compound C# identifiers (ICatalogQueryManager,
    // m_catalogQueryManager) are always found.  Auto-disable when regex/phrase is used.
    let use_substring = if use_regex || use_phrase {
        // If user explicitly asked for substring AND regex/phrase, that's a conflict
        if args.get("substring").and_then(|v| v.as_bool()) == Some(true) {
            return ToolCallResult::error(
                "substring is mutually exclusive with regex and phrase".to_string(),
            );
        }
        false
    } else {
        args.get("substring").and_then(|v| v.as_bool()).unwrap_or(true)
    };
    let context_lines = args.get("contextLines").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    // Auto-enable showLines when contextLines > 0 (BUG-6: contextLines without showLines was silently ignored)
    let show_lines = args.get("showLines").and_then(|v| v.as_bool()).unwrap_or(false)
        || context_lines > 0;
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

    // (Mutual exclusivity check is now handled above during use_substring init)

    // --- Substring: check if trigram index needs rebuild -----
    if use_substring {
        let needs_rebuild = ctx.index.read().map(|idx| idx.trigram_dirty).unwrap_or(false);
        if needs_rebuild {
            // Build trigram index under READ lock (doesn't block other readers)
            let new_trigram = ctx.index.read().ok().and_then(|idx| {
                if idx.trigram_dirty {
                    Some(build_trigram_index(&idx.index))
                } else {
                    None
                }
            });
            // Swap in under brief WRITE lock (microseconds, not ~200ms)
            if let Some(trigram) = new_trigram {
                if let Ok(mut idx) = ctx.index.write() {
                    if idx.trigram_dirty {  // double-check after acquiring write lock
                        eprintln!("[substring] Rebuilt trigram index: {} tokens, {} trigrams",
                            trigram.tokens.len(), trigram.trigram_map.len());
                        idx.trigram = trigram;
                        idx.trigram_dirty = false;
                    }
                }
            }
        }
    }

    let index = match ctx.index.read() {
        Ok(idx) => idx,
        Err(e) => return ToolCallResult::error(format!("Failed to acquire index lock: {}", e)),
    };

    // --- Substring search mode ------------------------------
    if use_substring {
        return handle_substring_search(ctx, &index, &terms_str, &ext_filter, &exclude_dir, &exclude,
            show_lines, context_lines, max_results, mode_and, count_only, search_start, &dir_filter);
    }

    // --- Phrase search mode ---------------------------------
    if use_phrase {
        return handle_phrase_search(
            &index, &terms_str, &ext_filter, &exclude_dir, &exclude,
            show_lines, context_lines, max_results, count_only, search_start, &dir_filter,
        );
    }

    // --- Normal token search --------------------------------
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
                let file_path = match index.files.get(posting.file_id as usize) {
                    Some(p) => p,
                    None => continue,
                };

                // Dir prefix filter (subdirectory search)
                if let Some(ref prefix) = dir_filter {
                    if !is_under_dir(file_path, prefix) { continue; }
                }

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
    dir_filter: &Option<String>,
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
    // Track which distinct term indices matched per file (for correct AND-mode filtering)
    let mut file_matched_terms: HashMap<u32, HashSet<usize>> = HashMap::new();

    for (term_idx, term) in raw_terms.iter().enumerate() {
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
                        // Trigram not found -> no candidates
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
                    let file_path = match index.files.get(posting.file_id as usize) {
                        Some(p) => p,
                        None => continue,
                    };

                    // Dir prefix filter (subdirectory search)
                    if let Some(prefix) = dir_filter {
                        if !is_under_dir(file_path, prefix) { continue; }
                    }

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
                    // Track distinct term index (not per-token) for correct AND filtering
                    file_matched_terms.entry(posting.file_id).or_default().insert(term_idx);
                }
            }
        }
    }

    // Dedup matched tokens
    all_matched_tokens.sort();
    all_matched_tokens.dedup();

    // Set terms_matched from the distinct matched term indices
    for (file_id, entry) in &mut file_scores {
        if let Some(matched) = file_matched_terms.get(file_id) {
            entry.terms_matched = matched.len();
        }
    }

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
    dir_filter: &Option<String>,
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
                    let path = match index.files.get(p.file_id as usize) {
                        Some(p) => p,
                        None => return false,
                    };
                    if let Some(prefix) = dir_filter {
                        if !is_under_dir(path, prefix) { return false; }
                    }
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

    // Step 2: Verify phrase match in raw file content.
    //
    // When the original phrase contains non-alphanumeric characters (XML tags,
    // angle brackets, etc.), the tokenizer strips them, causing false positives.
    // In that case, we match using the original phrase as a case-insensitive
    // substring against raw file content instead of the tokenized regex.
    // This eliminates false positives from tokenization stripping punctuation.
    let phrase_has_punctuation = phrase.chars().any(|c| !c.is_alphanumeric() && !c.is_whitespace());

    struct PhraseMatch {
        file_path: String,
        lines: Vec<u32>,
        content: Option<String>, // cached for show_lines to avoid re-reading
    }
    let mut results: Vec<PhraseMatch> = Vec::new();

    for &file_id in &candidates {
        let file_path = &index.files[file_id as usize];
        if let Ok(content) = std::fs::read_to_string(file_path) {
            let mut matching_lines = Vec::new();
            if phrase_has_punctuation {
                // Use raw phrase substring match (case-insensitive) to avoid
                // false positives from tokenizer stripping punctuation
                for (line_num, line) in content.lines().enumerate() {
                    if line.to_lowercase().contains(&phrase_lower) {
                        matching_lines.push((line_num + 1) as u32);
                    }
                }
            } else if phrase_re.is_match(&content) {
                // Use tokenized phrase regex (no punctuation → no false positives)
                for (line_num, line) in content.lines().enumerate() {
                    if phrase_re.is_match(line) {
                        matching_lines.push((line_num + 1) as u32);
                    }
                }
            }
            if !matching_lines.is_empty() {
                results.push(PhraseMatch {
                    file_path: file_path.clone(),
                    lines: matching_lines,
                    content: if show_lines { Some(content) } else { None },
                });
            }
        }
    }

    let total_files = results.len();
    let total_occurrences: usize = results.iter().map(|r| r.lines.len()).sum();

    // Sort by number of occurrences descending (most matches first)
    results.sort_by(|a, b| b.lines.len().cmp(&a.lines.len()));

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