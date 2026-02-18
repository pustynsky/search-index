//! search_definitions handler: AST-based code definition search.

use std::collections::HashMap;
use std::time::Instant;

use serde_json::{json, Value};

use crate::mcp::protocol::ToolCallResult;
use crate::definitions::{DefinitionEntry, DefinitionKind};

use super::utils::inject_body_into_obj;
use super::HandlerContext;

pub(crate) fn handle_search_definitions(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
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
    let max_results = args.get("maxResults")
        .and_then(|v| v.as_u64())
        .map(|v| if v == 0 { 100 } else { v })
        .unwrap_or(100) as usize;
    let exclude_dir: Vec<String> = args.get("excludeDir")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let include_body = args.get("includeBody").and_then(|v| v.as_bool()).unwrap_or(false);
    let max_body_lines = args.get("maxBodyLines").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
    let max_total_body_lines = args.get("maxTotalBodyLines").and_then(|v| v.as_u64()).unwrap_or(500) as usize;

    // --- containsLine: find containing method/class by line number ---
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
    let mut candidates = candidate_indices.unwrap_or_else(|| {
        (0..index.definitions.len() as u32).collect()
    });

    // Deduplicate candidate indices (a definition may appear multiple times
    // if e.g. multiple attributes normalize to the same name)
    candidates.sort_unstable();
    candidates.dedup();

    // Apply remaining filters (file, parent, excludeDir) on actual entries
    let mut results: Vec<&DefinitionEntry> = candidates.iter()
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