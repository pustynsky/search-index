//! search_callers handler: call tree building (up/down).

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

use serde_json::{json, Value};

use crate::mcp::protocol::ToolCallResult;
use crate::ContentIndex;
use crate::definitions::{CallSite, DefinitionEntry, DefinitionIndex, DefinitionKind};
use search::generate_trigrams;

use super::HandlerContext;

pub(crate) fn handle_search_callers(ctx: &HandlerContext, args: &Value) -> ToolCallResult {
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
    let node_count = AtomicUsize::new(0);

    // Check for ambiguous method names and generate warning
    let method_lower = method_name.to_lowercase();
    let mut ambiguity_warning: Option<String> = None;
    if class_filter.is_none() {
        if let Some(name_indices) = def_idx.name_index.get(&method_lower) {
            let method_defs: Vec<&DefinitionEntry> = name_indices.iter()
                .filter_map(|&di| def_idx.definitions.get(di as usize))
                .filter(|d| d.kind == DefinitionKind::Method || d.kind == DefinitionKind::Constructor || d.kind == DefinitionKind::Function)
                .collect();

            let unique_classes: HashSet<&str> = method_defs.iter()
                .filter_map(|d| d.parent.as_deref())
                .collect();

            if unique_classes.len() > 1 {
                let total = unique_classes.len();
                let mut class_list: Vec<&str> = unique_classes.into_iter().collect();
                class_list.sort_unstable();
                const MAX_LISTED: usize = 10;
                if total <= MAX_LISTED {
                    ambiguity_warning = Some(format!(
                        "Method '{}' found in {} classes: {}. Results may mix callers from different classes. Use 'class' parameter to scope the search.",
                        method_name, total, class_list.join(", ")
                    ));
                } else {
                    let shown: Vec<&str> = class_list.into_iter().take(MAX_LISTED).collect();
                    ambiguity_warning = Some(format!(
                        "Method '{}' found in {} classes (showing first {}): {}… Use 'class' parameter to scope the search.",
                        method_name, total, MAX_LISTED, shown.join(", ")
                    ));
                }
            }
        }
    }

    if direction == "up" {
        let mut visited: HashSet<String> = HashSet::new();
        let tree = build_caller_tree(
            &method_name,
            class_filter.as_deref(),
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
            &mut HashSet::new(),
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

// ─── Internal helpers ───────────────────────────────────────────────

/// Remove duplicate nodes from the caller tree (can occur with resolveInterfaces
/// when the same caller is found through multiple interface implementations).
fn dedup_caller_tree(tree: Vec<Value>) -> Vec<Value> {
    let mut seen: HashSet<String> = HashSet::new();
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
pub(crate) fn find_containing_method(
    def_idx: &DefinitionIndex,
    file_id: u32,
    line: u32,
) -> Option<(String, Option<String>, u32)> {
    let def_indices = def_idx.file_index.get(&file_id)?;

    let mut best: Option<&DefinitionEntry> = None;
    for &di in def_indices {
        if let Some(def) = def_idx.definitions.get(di as usize) {
            match def.kind {
                DefinitionKind::Method | DefinitionKind::Constructor | DefinitionKind::Property | DefinitionKind::Function => {}
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

/// Collect file_ids from the content index where `term` appears as a SUBSTRING of
/// another token. Uses the trigram index for fast O(k) lookup.
/// Handles field naming patterns like m_catalogQueryManager, _catalogQueryManager, etc.
/// No-op if the trigram index is empty or the term is shorter than 3 chars.
fn collect_substring_file_ids(
    term: &str,
    content_index: &ContentIndex,
    file_ids: &mut HashSet<u32>,
) {
    if term.len() < 3 {
        return; // trigrams require at least 3 chars
    }
    let trigram_idx = &content_index.trigram;
    if trigram_idx.tokens.is_empty() {
        return; // trigram index not built yet
    }

    let trigrams = generate_trigrams(term);
    if trigrams.is_empty() {
        return;
    }

    // Intersect trigram posting lists to find candidate token indices
    let mut candidates: Option<Vec<u32>> = None;
    for tri in &trigrams {
        if let Some(posting_list) = trigram_idx.trigram_map.get(tri) {
            candidates = Some(match candidates {
                None => posting_list.clone(),
                Some(prev) => {
                    let set: HashSet<u32> = prev.into_iter().collect();
                    posting_list.iter().filter(|&&x| set.contains(&x)).copied().collect()
                }
            });
        } else {
            // Trigram not found → no tokens can contain this term
            return;
        }
    }

    // Verify candidates actually contain the term, then collect their file_ids
    if let Some(candidate_indices) = candidates {
        for &ti in &candidate_indices {
            if let Some(tok) = trigram_idx.tokens.get(ti as usize) {
                // Only match tokens strictly LONGER than the term (substring, not exact)
                if tok.len() > term.len() && tok.contains(term) {
                    if let Some(postings) = content_index.index.get(tok) {
                        file_ids.extend(postings.iter().map(|p| p.file_id));
                    }
                }
            }
        }
    }
}

/// Build a caller tree recursively (direction = "up").
/// `parent_class` is used to disambiguate common method names -- when recursing,
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
    visited: &mut HashSet<String>,
    limits: &CallerLimits,
    node_count: &AtomicUsize,
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
    let parent_file_ids: Option<HashSet<u32>> = parent_class.and_then(|cls| {
        let cls_lower = cls.to_lowercase();
        let mut file_ids: HashSet<u32> = HashSet::new();

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

        // Trigram substring matching: find files where class name appears as a
        // SUBSTRING of another token (e.g. m_catalogQueryManager, _catalogQueryManager).
        // Uses the trigram index for O(k) lookup instead of O(n) linear scan.
        collect_substring_file_ids(&cls_lower, content_index, &mut file_ids);
        collect_substring_file_ids(&interface_name, content_index, &mut file_ids);

        if file_ids.is_empty() { None } else { Some(file_ids) }
    });

    let mut callers: Vec<Value> = Vec::new();
    let mut seen_callers: HashSet<String> = HashSet::new();

    let mut definition_locations: HashSet<(u32, u32)> = HashSet::new();
    if let Some(name_indices) = def_idx.name_index.get(&method_lower) {
        for &di in name_indices {
            if let Some(def) = def_idx.definitions.get(di as usize)
                && (def.kind == DefinitionKind::Method || def.kind == DefinitionKind::Constructor || def.kind == DefinitionKind::Function) {
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
            .is_some_and(|e| {
                ext_filter.split(',')
                    .any(|allowed| e.eq_ignore_ascii_case(allowed.trim()))
            });
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
    visited: &mut HashSet<String>,
    limits: &CallerLimits,
    node_count: &AtomicUsize,
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
                            let kind_ok = d.kind == DefinitionKind::Method || d.kind == DefinitionKind::Constructor || d.kind == DefinitionKind::Function;
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
    let mut seen_callees: HashSet<String> = HashSet::new();

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
                    .is_some_and(|e| {
                        ext_filter.split(',')
                            .any(|allowed| e.eq_ignore_ascii_case(allowed.trim()))
                    });
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
pub(crate) fn resolve_call_site(call: &CallSite, def_idx: &DefinitionIndex) -> Vec<u32> {
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

        // Only match methods, constructors, and functions
        if def.kind != DefinitionKind::Method && def.kind != DefinitionKind::Constructor && def.kind != DefinitionKind::Function {
            continue;
        }

        if let Some(ref recv_type) = call.receiver_type {
            // We have receiver type info -- use it to disambiguate
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
            }
        } else {
            // No receiver type -- accept any matching method/constructor
            // (this handles simple calls like Foo() within the same class)
            resolved.push(di);
        }
    }

    resolved
}