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
/// Returns `(name, parent, line_start, definition_index)`.
pub(crate) fn find_containing_method(
    def_idx: &DefinitionIndex,
    file_id: u32,
    line: u32,
) -> Option<(String, Option<String>, u32, u32)> {
    let def_indices = def_idx.file_index.get(&file_id)?;

    let mut best: Option<(u32, &DefinitionEntry)> = None;
    for &di in def_indices {
        if let Some(def) = def_idx.definitions.get(di as usize) {
            match def.kind {
                DefinitionKind::Method | DefinitionKind::Constructor | DefinitionKind::Property | DefinitionKind::Function => {}
                _ => continue,
            }
            if def.line_start <= line && def.line_end >= line {
                if let Some((_, current_best)) = best {
                    if (def.line_end - def.line_start) < (current_best.line_end - current_best.line_start) {
                        best = Some((di, def));
                    }
                } else {
                    best = Some((di, def));
                }
            }
        }
    }

    best.map(|(di, d)| (d.name.clone(), d.parent.clone(), d.line_start, di))
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

/// Verifies that a call on a specific line actually targets the expected class.
/// Uses pre-computed call-site data from the definition index.
///
/// Returns true if:
/// - The call-site has a receiver_type matching target_class (direct, interface I-prefix, or inheritance)
/// - The call-site has no receiver_type AND the caller is in the same class or inherits from target
/// - No call-site data exists (graceful fallback — don't filter what we can't verify)
/// - target_class is None (no filtering needed)
///
/// Returns false if:
/// - The call-site has a receiver_type that does NOT match target_class
fn verify_call_site_target(
    def_idx: &DefinitionIndex,
    caller_di: u32,
    call_line: u32,
    method_name: &str,
    target_class: Option<&str>,
) -> bool {
    // If no target class specified, accept everything
    let target_class = match target_class {
        Some(tc) => tc,
        None => return true,
    };

    // Get call sites for the caller method from the definition index
    let call_sites = match def_idx.method_calls.get(&caller_di) {
        Some(cs) => cs,
        None => return false, // no call-site data → reject (parser covers all patterns now)
    };

    // Find call sites on the specified line with the matching method name
    let method_name_lower = method_name.to_lowercase();
    let matching_calls: Vec<&CallSite> = call_sites
        .iter()
        .filter(|cs| cs.line == call_line && cs.method_name.to_lowercase() == method_name_lower)
        .collect();

    // If no call-site data found on this line:
    // Method has call-site data but no call at this line →
    // content index matched a comment or non-code text → filter out
    if matching_calls.is_empty() {
        return call_sites.is_empty(); // true only if method has zero call data (shouldn't happen but safe)
    }

    let target_lower = target_class.to_lowercase();
    // Also prepare interface variant: "IFoo" for "Foo"
    let target_interface = format!("i{}", target_lower);

    // Get the caller method's definition to check parent class
    let caller_def = match def_idx.definitions.get(caller_di as usize) {
        Some(d) => d,
        None => return true,
    };
    let caller_parent = caller_def.parent.as_deref();

    // Get target class's base_types for inheritance check
    let target_base_types: Vec<String> = def_idx
        .name_index
        .get(&target_lower)
        .map(|indices| {
            indices
                .iter()
                .filter_map(|&di| def_idx.definitions.get(di as usize))
                .filter(|d| {
                    matches!(
                        d.kind,
                        DefinitionKind::Class | DefinitionKind::Struct | DefinitionKind::Record
                    )
                })
                .flat_map(|d| d.base_types.iter().map(|bt| bt.to_lowercase()))
                .collect()
        })
        .unwrap_or_default();

    // Check if ANY matching call-site passes verification
    for cs in &matching_calls {
        match &cs.receiver_type {
            Some(rt) => {
                let rt_lower = rt.to_lowercase();
                // Direct match
                if rt_lower == target_lower {
                    return true;
                }
                // Interface match: receiver is IFoo, target is Foo
                if rt_lower == target_interface {
                    return true;
                }
                // Reverse interface: receiver is Foo, target is IFoo
                if target_lower.starts_with('i')
                    && rt_lower == target_lower[1..]
                {
                    return true;
                }
                // Inheritance: target class has base_types containing the receiver_type
                if target_base_types.iter().any(|bt| *bt == rt_lower) {
                    return true;
                }
            }
            None => {
                // No receiver type — accept if caller is in the same class or a subclass
                if let Some(cp) = caller_parent {
                    let cp_lower = cp.to_lowercase();
                    if cp_lower == target_lower || cp_lower == target_interface {
                        return true;
                    }
                    // Check if caller's class inherits from target
                    let caller_inherits = def_idx
                        .name_index
                        .get(&cp_lower)
                        .map(|indices| {
                            indices.iter().any(|&di| {
                                def_idx
                                    .definitions
                                    .get(di as usize)
                                    .is_some_and(|d| {
                                        matches!(
                                            d.kind,
                                            DefinitionKind::Class
                                                | DefinitionKind::Struct
                                                | DefinitionKind::Record
                                        ) && d
                                            .base_types
                                            .iter()
                                            .any(|bt| bt.to_lowercase() == target_lower)
                                    })
                            })
                        })
                        .unwrap_or(false);
                    if caller_inherits {
                        return true;
                    }
                }
                // No receiver + different class + no inheritance → false positive
            }
        }
    }

    false
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

        // Note: we intentionally do NOT expand pre-filter by base_types (interfaces).
        // Common interfaces (IDisposable, IEnumerable, etc.) would add thousands of
        // irrelevant files, disabling the pre-filter. Inheritance is already verified
        // in verify_call_site_target() via target_base_types.

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

            if let Some((caller_name, caller_parent, caller_line, caller_di)) =
                find_containing_method(def_idx, def_fid, line)
            {
                // Verify the call on this line actually targets the expected class
                // using pre-computed call-site data from the AST
                if parent_class.is_some() {
                    if !verify_call_site_target(
                        def_idx,
                        caller_di,
                        line,
                        &method_lower,
                        parent_class,
                    ) {
                        continue;
                    }
                }

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
            let caller_parent = def_idx.definitions.get(method_di as usize)
                .and_then(|d| d.parent.as_deref());
            let resolved = resolve_call_site(call, def_idx, caller_parent);

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
                    callee_def.parent.as_deref(), // scope recursion to the callee's own class
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

/// Check if a class (by lowercased name) has generic parameters in its signature.
/// Returns true if ANY class definition with that name has `<` in its signature.
fn is_class_generic(def_idx: &DefinitionIndex, class_name_lower: &str) -> bool {
    if let Some(indices) = def_idx.name_index.get(class_name_lower) {
        for &di in indices {
            if let Some(def) = def_idx.definitions.get(di as usize) {
                if matches!(def.kind, DefinitionKind::Class | DefinitionKind::Struct | DefinitionKind::Record | DefinitionKind::Interface) {
                    if let Some(ref sig) = def.signature {
                        if sig.contains('<') {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Resolve a CallSite to actual definition indices in the definition index.
/// Uses receiver_type to disambiguate when available. When receiver is unknown,
/// scopes to the caller's own class if `caller_parent` is provided, otherwise
/// falls back to accepting all matching methods.
pub(crate) fn resolve_call_site(call: &CallSite, def_idx: &DefinitionIndex, caller_parent: Option<&str>) -> Vec<u32> {
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
                    // Generic arity check: if call site is generic (e.g. new List<int>())
                    // but the resolved class is NOT generic, skip — likely BCL name collision
                    if call.receiver_is_generic && !is_class_generic(def_idx, &parent_lower) {
                        continue;
                    }
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
            // No receiver type -- prefer methods in the same class as the caller
            if let Some(caller_cls) = caller_parent {
                if let Some(ref parent) = def.parent {
                    if parent.eq_ignore_ascii_case(caller_cls) {
                        resolved.push(di);
                    }
                }
            } else {
                // No caller class context -- accept all (backward-compatible)
                resolved.push(di);
            }
        }
    }

    resolved
}

// ─── Unit tests for verify_call_site_target ─────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::{CallSite, DefinitionEntry, DefinitionIndex, DefinitionKind};
    use std::collections::HashMap;


    /// Helper: build a minimal DefinitionIndex with given definitions and method_calls.
    fn make_def_index(
        definitions: Vec<DefinitionEntry>,
        method_calls: HashMap<u32, Vec<CallSite>>,
    ) -> DefinitionIndex {
        let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
        let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
        let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();

        for (i, def) in definitions.iter().enumerate() {
            let idx = i as u32;
            name_index.entry(def.name.to_lowercase()).or_default().push(idx);
            kind_index.entry(def.kind).or_default().push(idx);
            file_index.entry(def.file_id).or_default().push(idx);
        }

        DefinitionIndex {
            root: ".".to_string(),
            created_at: 0,
            extensions: vec!["ts".to_string()],
            files: vec!["src/OrderController.ts".to_string(), "src/OrderValidator.ts".to_string()],
            definitions,
            name_index,
            kind_index,
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index,
            path_to_id: HashMap::new(),
            method_calls,
            parse_errors: 0,
            lossy_file_count: 0,
            empty_file_ids: Vec::new(),
        }
    }

    /// Helper: create a DefinitionEntry for a class.
    fn class_def(file_id: u32, name: &str, base_types: Vec<&str>) -> DefinitionEntry {
        DefinitionEntry {
            file_id,
            name: name.to_string(),
            kind: DefinitionKind::Class,
            line_start: 1,
            line_end: 100,
            parent: None,
            signature: None,
            modifiers: vec![],
            attributes: vec![],
            base_types: base_types.into_iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Helper: create a DefinitionEntry for a method inside a class.
    fn method_def(file_id: u32, name: &str, parent: &str, line_start: u32, line_end: u32) -> DefinitionEntry {
        DefinitionEntry {
            file_id,
            name: name.to_string(),
            kind: DefinitionKind::Method,
            line_start,
            line_end,
            parent: Some(parent.to_string()),
            signature: None,
            modifiers: vec![],
            attributes: vec![],
            base_types: vec![],
        }
    }

    // ─── Test 1: Direct receiver match ──────────────────────────────

    #[test]
    fn test_verify_call_site_target_direct_match() {
        // OrderController.processOrder() calls validator.validate() at line 25
        // receiver_type = "OrderValidator"
        let definitions = vec![
            class_def(0, "OrderController", vec![]),              // idx 0
            method_def(0, "processOrder", "OrderController", 20, 40), // idx 1
            class_def(1, "OrderValidator", vec![]),                // idx 2
            method_def(1, "validate", "OrderValidator", 10, 30),  // idx 3
        ];

        let mut method_calls = HashMap::new();
        method_calls.insert(1u32, vec![
            CallSite {
                method_name: "validate".to_string(),
                receiver_type: Some("OrderValidator".to_string()),
                line: 25,
                receiver_is_generic: false,
            },
        ]);

        let def_idx = make_def_index(definitions, method_calls);

        // caller_di=1 (processOrder), call_line=25, method="validate", target="OrderValidator"
        assert!(verify_call_site_target(&def_idx, 1, 25, "validate", Some("OrderValidator")));
    }

    // ─── Test 2: Different receiver → should reject ─────────────────

    #[test]
    fn test_verify_call_site_target_different_receiver() {
        // OrderController.processOrder() calls path.resolve() at line 25
        // receiver_type = "Path" — target is "DependencyTask", should NOT match
        let definitions = vec![
            class_def(0, "OrderController", vec![]),              // idx 0
            method_def(0, "processOrder", "OrderController", 20, 40), // idx 1
            class_def(1, "DependencyTask", vec![]),               // idx 2
            method_def(1, "resolve", "DependencyTask", 10, 30),  // idx 3
        ];

        let mut method_calls = HashMap::new();
        method_calls.insert(1u32, vec![
            CallSite {
                method_name: "resolve".to_string(),
                receiver_type: Some("Path".to_string()),
                line: 25,
                receiver_is_generic: false,
            },
        ]);

        let def_idx = make_def_index(definitions, method_calls);

        // receiver is "Path" but target class is "DependencyTask" — should return false
        assert!(!verify_call_site_target(&def_idx, 1, 25, "resolve", Some("DependencyTask")));
    }

    // ─── Test 3: No receiver, same class (implicit this) ────────────

    #[test]
    fn test_verify_call_site_target_no_receiver_same_class() {
        // OrderValidator.check() calls this.validate() at line 55
        // receiver_type = None (implicit this), caller is in OrderValidator
        let definitions = vec![
            class_def(1, "OrderValidator", vec![]),                // idx 0
            method_def(1, "check", "OrderValidator", 50, 70),     // idx 1
            method_def(1, "validate", "OrderValidator", 10, 30),  // idx 2
        ];

        let mut method_calls = HashMap::new();
        method_calls.insert(1u32, vec![
            CallSite {
                method_name: "validate".to_string(),
                receiver_type: None,
                line: 55,
                receiver_is_generic: false,
            },
        ]);

        let def_idx = make_def_index(definitions, method_calls);

        // caller is in OrderValidator, target is OrderValidator, no receiver → true
        assert!(verify_call_site_target(&def_idx, 1, 55, "validate", Some("OrderValidator")));
    }

    // ─── Test 4: No receiver, different class ───────────────────────

    #[test]
    fn test_verify_call_site_target_no_receiver_different_class() {
        // OrderController.processOrder() calls validate() at line 25
        // receiver_type = None, caller is in OrderController, target is OrderValidator
        let definitions = vec![
            class_def(0, "OrderController", vec![]),              // idx 0
            method_def(0, "processOrder", "OrderController", 20, 40), // idx 1
            class_def(1, "OrderValidator", vec![]),                // idx 2
            method_def(1, "validate", "OrderValidator", 10, 30),  // idx 3
        ];

        let mut method_calls = HashMap::new();
        method_calls.insert(1u32, vec![
            CallSite {
                method_name: "validate".to_string(),
                receiver_type: None,
                line: 25,
                receiver_is_generic: false,
            },
        ]);

        let def_idx = make_def_index(definitions, method_calls);

        // caller is in OrderController, target is OrderValidator, no receiver → false
        assert!(!verify_call_site_target(&def_idx, 1, 25, "validate", Some("OrderValidator")));
    }

    // ─── Test 5: No target class → always accept ────────────────────

    #[test]
    fn test_verify_call_site_target_no_target_class() {
        let definitions = vec![
            class_def(0, "OrderController", vec![]),              // idx 0
            method_def(0, "processOrder", "OrderController", 20, 40), // idx 1
        ];

        let mut method_calls = HashMap::new();
        method_calls.insert(1u32, vec![
            CallSite {
                method_name: "validate".to_string(),
                receiver_type: Some("SomeRandomClass".to_string()),
                line: 25,
                receiver_is_generic: false,
            },
        ]);

        let def_idx = make_def_index(definitions, method_calls);

        // target_class = None → should always return true (no filtering)
        assert!(verify_call_site_target(&def_idx, 1, 25, "validate", None));
    }

    // ─── Test 6: No call-site data → graceful fallback (true) ───────

    #[test]
    fn test_verify_call_site_target_no_call_site_data() {
        let definitions = vec![
            class_def(0, "OrderController", vec![]),              // idx 0
            method_def(0, "processOrder", "OrderController", 20, 40), // idx 1
        ];

        // Empty method_calls — no call-site data for any method
        let method_calls = HashMap::new();

        let def_idx = make_def_index(definitions, method_calls);

        // No call-site data → rejection (parser covers all patterns now)
        assert!(!verify_call_site_target(&def_idx, 1, 25, "validate", Some("OrderValidator")));
    }

    // ─── Test 7: Interface match (IOrderValidator → OrderValidator) ─

    #[test]
    fn test_verify_call_site_target_interface_match() {
        // OrderController.processOrder() calls validator.validate() at line 25
        // receiver_type = "IOrderValidator", target_class = "OrderValidator"
        // Should match via interface I-prefix convention
        let definitions = vec![
            class_def(0, "OrderController", vec![]),              // idx 0
            method_def(0, "processOrder", "OrderController", 20, 40), // idx 1
            class_def(1, "OrderValidator", vec!["IOrderValidator"]), // idx 2
            method_def(1, "validate", "OrderValidator", 10, 30),  // idx 3
        ];

        let mut method_calls = HashMap::new();
        method_calls.insert(1u32, vec![
            CallSite {
                method_name: "validate".to_string(),
                receiver_type: Some("IOrderValidator".to_string()),
                line: 25,
                receiver_is_generic: false,
            },
        ]);

        let def_idx = make_def_index(definitions, method_calls);

        // receiver is "IOrderValidator", target is "OrderValidator" → should match via I-prefix
        assert!(verify_call_site_target(&def_idx, 1, 25, "validate", Some("OrderValidator")));
    }

    // ─── Test 8: Comment line — method has call sites but not at queried line ─

    #[test]
    fn test_verify_call_site_target_comment_line_not_real_call() {
        // OrderController.processOrder() has a call to endsWith() at line 10
        // but we query for "resolve" at line 5 where no call site exists
        // → content index matched a comment or non-code text → should return false
        let definitions = vec![
            class_def(0, "OrderController", vec![]),              // idx 0
            method_def(0, "processOrder", "OrderController", 1, 20), // idx 1
        ];

        let mut method_calls = HashMap::new();
        method_calls.insert(1u32, vec![
            CallSite {
                method_name: "endsWith".to_string(),
                receiver_type: Some("String".to_string()),
                line: 10,
                receiver_is_generic: false,
            },
        ]);

        let def_idx = make_def_index(definitions, method_calls);

        // Method has call-site data (endsWith at line 10), but no call at line 5
        // → this is a false positive from content index → should return false
        assert!(!verify_call_site_target(&def_idx, 1, 5, "resolve", Some("PathUtils")));
    }

    // ─── Test 9: Pre-filter does NOT expand by base_types ────────────

    #[test]
    fn test_prefilter_does_not_expand_by_base_types() {
        // Scenario:
        // - ResourceManager implements IDisposable, has method Dispose
        // - Many files mention "idisposable" (simulating a large codebase)
        // - Only one file actually calls resourceManager.Dispose()
        // - The pre-filter should NOT include all IDisposable files
        //
        // We test this by running build_caller_tree and verifying that
        // only the file with the actual call is in the results.

        use crate::{ContentIndex, Posting, TrigramIndex};
        use std::sync::atomic::AtomicUsize;
        use std::path::PathBuf;

        // --- Definition Index ---
        // file 0: ResourceManager.cs (defines ResourceManager : IDisposable + Dispose)
        // file 1: Caller.cs (calls resourceManager.Dispose())
        // files 2..11: IDisposable-mentioning files (no actual Dispose call on ResourceManager)

        let definitions = vec![
            // idx 0: class ResourceManager : IDisposable
            DefinitionEntry {
                file_id: 0,
                name: "ResourceManager".to_string(),
                kind: DefinitionKind::Class,
                line_start: 1,
                line_end: 50,
                parent: None,
                signature: None,
                modifiers: vec![],
                attributes: vec![],
                base_types: vec!["IDisposable".to_string()],
            },
            // idx 1: method ResourceManager.Dispose
            DefinitionEntry {
                file_id: 0,
                name: "Dispose".to_string(),
                kind: DefinitionKind::Method,
                line_start: 10,
                line_end: 20,
                parent: Some("ResourceManager".to_string()),
                signature: None,
                modifiers: vec![],
                attributes: vec![],
                base_types: vec![],
            },
            // idx 2: class Caller (in file 1)
            DefinitionEntry {
                file_id: 1,
                name: "Caller".to_string(),
                kind: DefinitionKind::Class,
                line_start: 1,
                line_end: 30,
                parent: None,
                signature: None,
                modifiers: vec![],
                attributes: vec![],
                base_types: vec![],
            },
            // idx 3: method Caller.DoWork (contains the actual call)
            DefinitionEntry {
                file_id: 1,
                name: "DoWork".to_string(),
                kind: DefinitionKind::Method,
                line_start: 5,
                line_end: 25,
                parent: Some("Caller".to_string()),
                signature: None,
                modifiers: vec![],
                attributes: vec![],
                base_types: vec![],
            },
        ];

        // Call site: Caller.DoWork calls resourceManager.Dispose() at line 15
        let mut method_calls: HashMap<u32, Vec<CallSite>> = HashMap::new();
        method_calls.insert(3, vec![
            CallSite {
                method_name: "Dispose".to_string(),
                receiver_type: Some("ResourceManager".to_string()),
                line: 15,
                receiver_is_generic: false,
            },
        ]);

        // Build def index
        let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
        let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
        let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();

        for (i, def) in definitions.iter().enumerate() {
            let idx = i as u32;
            name_index.entry(def.name.to_lowercase()).or_default().push(idx);
            kind_index.entry(def.kind).or_default().push(idx);
            file_index.entry(def.file_id).or_default().push(idx);
        }

        let num_files = 12u32; // file 0 + file 1 + 10 IDisposable files
        let mut files_list: Vec<String> = vec![
            "src/ResourceManager.cs".to_string(),
            "src/Caller.cs".to_string(),
        ];
        path_to_id.insert(PathBuf::from("src/ResourceManager.cs"), 0);
        path_to_id.insert(PathBuf::from("src/Caller.cs"), 1);

        for i in 2..num_files {
            let path = format!("src/Service{}.cs", i);
            files_list.push(path.clone());
            path_to_id.insert(PathBuf::from(&path), i);
        }

        let def_idx = DefinitionIndex {
            root: ".".to_string(),
            created_at: 0,
            extensions: vec!["cs".to_string()],
            files: files_list.clone(),
            definitions,
            name_index,
            kind_index,
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index,
            path_to_id,
            method_calls,
            parse_errors: 0,
            lossy_file_count: 0,
            empty_file_ids: Vec::new(),
        };

        // --- Content Index ---
        // Token "dispose" appears in file 0 (definition) and file 1 (actual call)
        // Token "idisposable" appears in files 2..11 (many files mentioning the interface)
        // Token "resourcemanager" appears only in file 0 and file 1
        let mut index: HashMap<String, Vec<Posting>> = HashMap::new();

        // "dispose" in file 0 (definition, line 10) and file 1 (call, line 15)
        index.insert("dispose".to_string(), vec![
            Posting { file_id: 0, lines: vec![10] },
            Posting { file_id: 1, lines: vec![15] },
        ]);

        // "resourcemanager" in file 0 and file 1
        index.insert("resourcemanager".to_string(), vec![
            Posting { file_id: 0, lines: vec![1] },
            Posting { file_id: 1, lines: vec![15] },
        ]);

        // "idisposable" in many files (simulating common interface)
        let idisposable_postings: Vec<Posting> = (2..num_files)
            .map(|fid| Posting { file_id: fid, lines: vec![1, 5, 10] })
            .collect();
        index.insert("idisposable".to_string(), idisposable_postings);

        let content_index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: files_list,
            index,
            total_tokens: 100,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![50; num_files as usize],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: None,
            path_to_id: None,
        };

        // --- Run build_caller_tree ---
        let mut visited = HashSet::new();
        let limits = CallerLimits {
            max_callers_per_level: 50,
            max_total_nodes: 200,
        };
        let node_count = AtomicUsize::new(0);

        let callers = build_caller_tree(
            "Dispose",
            Some("ResourceManager"),
            3,
            0,
            &content_index,
            &def_idx,
            "cs",
            &[],
            &[],
            false, // no interface resolution for this test
            &mut visited,
            &limits,
            &node_count,
        );

        // Should find exactly one caller: Caller.DoWork
        assert_eq!(callers.len(), 1, "Expected exactly 1 caller, got {}: {:?}", callers.len(), callers);
        let caller = &callers[0];
        assert_eq!(caller["method"].as_str().unwrap(), "DoWork");
        assert_eq!(caller["class"].as_str().unwrap(), "Caller");

        // Verify no false positives from IDisposable files
        let caller_file = caller["file"].as_str().unwrap();
        assert_eq!(caller_file, "Caller.cs", "Caller should be from Caller.cs, not an IDisposable file");
    }

    // ─── Test 10: resolve_call_site scopes by caller_parent when no receiver_type ──

    #[test]
    fn test_resolve_call_site_scopes_by_caller_parent() {
        let definitions = vec![
            class_def(0, "ClassA", vec![]),
            method_def(0, "doWork", "ClassA", 5, 15),
            class_def(1, "ClassB", vec![]),
            method_def(1, "doWork", "ClassB", 5, 15),
        ];
        let def_idx = make_def_index(definitions, HashMap::new());
        let call = CallSite {
            method_name: "doWork".to_string(),
            receiver_type: None,
            line: 10,
            receiver_is_generic: false,
        };

        let resolved_a = resolve_call_site(&call, &def_idx, Some("ClassA"));
        assert_eq!(resolved_a.len(), 1);
        assert_eq!(def_idx.definitions[resolved_a[0] as usize].parent.as_deref(), Some("ClassA"));

        let resolved_b = resolve_call_site(&call, &def_idx, Some("ClassB"));
        assert_eq!(resolved_b.len(), 1);
        assert_eq!(def_idx.definitions[resolved_b[0] as usize].parent.as_deref(), Some("ClassB"));

        let resolved_all = resolve_call_site(&call, &def_idx, None);
        assert_eq!(resolved_all.len(), 2);
    }

    // ─── Test 11: build_callee_tree depth=2 no cross-class pollution ──

    #[test]
    fn test_callee_tree_depth2_no_cross_class_pollution() {
        use std::sync::atomic::AtomicUsize;

        let definitions = vec![
            DefinitionEntry { file_id: 0, name: "ClassA".to_string(), kind: DefinitionKind::Class, line_start: 1, line_end: 50, parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
            DefinitionEntry { file_id: 0, name: "process".to_string(), kind: DefinitionKind::Method, line_start: 5, line_end: 20, parent: Some("ClassA".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
            DefinitionEntry { file_id: 0, name: "internalWork".to_string(), kind: DefinitionKind::Method, line_start: 22, line_end: 30, parent: Some("ClassA".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
            DefinitionEntry { file_id: 1, name: "Helper".to_string(), kind: DefinitionKind::Class, line_start: 1, line_end: 40, parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
            DefinitionEntry { file_id: 1, name: "run".to_string(), kind: DefinitionKind::Method, line_start: 5, line_end: 20, parent: Some("Helper".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
            DefinitionEntry { file_id: 1, name: "helperStep".to_string(), kind: DefinitionKind::Method, line_start: 22, line_end: 35, parent: Some("Helper".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
            DefinitionEntry { file_id: 2, name: "ClassB".to_string(), kind: DefinitionKind::Class, line_start: 1, line_end: 40, parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
            DefinitionEntry { file_id: 2, name: "internalWork".to_string(), kind: DefinitionKind::Method, line_start: 5, line_end: 15, parent: Some("ClassB".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
            DefinitionEntry { file_id: 2, name: "helperStep".to_string(), kind: DefinitionKind::Method, line_start: 17, line_end: 30, parent: Some("ClassB".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
        ];

        let mut method_calls: HashMap<u32, Vec<CallSite>> = HashMap::new();
        method_calls.insert(1, vec![
            CallSite { method_name: "run".to_string(), receiver_type: Some("Helper".to_string()), line: 10, receiver_is_generic: false },
            CallSite { method_name: "internalWork".to_string(), receiver_type: None, line: 15, receiver_is_generic: false },
        ]);
        method_calls.insert(4, vec![
            CallSite { method_name: "helperStep".to_string(), receiver_type: None, line: 12, receiver_is_generic: false },
        ]);

        let def_idx = make_def_index(definitions, method_calls);
        let mut visited = HashSet::new();
        let limits = CallerLimits { max_callers_per_level: 50, max_total_nodes: 200 };
        let node_count = AtomicUsize::new(0);

        let callees = build_callee_tree("process", Some("ClassA"), 3, 0, &def_idx, "ts", &[], &[], &mut visited, &limits, &node_count);

        assert_eq!(callees.len(), 2, "Should have 2 callees, got {:?}", callees);
        let callee_names: Vec<(&str, &str)> = callees.iter()
            .map(|c| (c["method"].as_str().unwrap(), c["class"].as_str().unwrap_or("?")))
            .collect();
        assert!(callee_names.contains(&("run", "Helper")));
        assert!(callee_names.contains(&("internalWork", "ClassA")));
        assert!(!callee_names.contains(&("internalWork", "ClassB")), "ClassB.internalWork should NOT appear");

        let run_node = callees.iter().find(|c| c["method"] == "run").unwrap();
        let run_callees = run_node["callees"].as_array().unwrap();
        assert_eq!(run_callees.len(), 1);
        assert_eq!(run_callees[0]["method"].as_str().unwrap(), "helperStep");
        assert_eq!(run_callees[0]["class"].as_str().unwrap(), "Helper", "helperStep should be Helper, not ClassB");
    }

    // ─── Test 12: Generic arity mismatch filters out non-generic class ──

    #[test]
    fn test_resolve_call_site_generic_arity_mismatch() {
        // Scenario: new List<int>() should NOT resolve to a non-generic List class
        // that happens to have the same name (e.g. ReportRenderingModel.List : DataRegion)
        let definitions = vec![
            // idx 0: non-generic List class (user-defined, NOT System.Collections.Generic.List<T>)
            DefinitionEntry {
                file_id: 0, name: "List".to_string(), kind: DefinitionKind::Class,
                line_start: 1, line_end: 50, parent: None,
                signature: Some("internal sealed class List : DataRegion".to_string()),
                modifiers: vec![], attributes: vec![], base_types: vec!["DataRegion".to_string()],
            },
            // idx 1: constructor of non-generic List
            DefinitionEntry {
                file_id: 0, name: "List".to_string(), kind: DefinitionKind::Constructor,
                line_start: 10, line_end: 20, parent: Some("List".to_string()),
                signature: Some("internal List(int, ReportProcessing.List, ListInstance, RenderingContext)".to_string()),
                modifiers: vec![], attributes: vec![], base_types: vec![],
            },
        ];

        let def_idx = make_def_index(definitions, HashMap::new());

        // Call site: new List<CatalogEntry>() — generic call
        let call_generic = CallSite {
            method_name: "List".to_string(),
            receiver_type: Some("List".to_string()),
            line: 252,
            receiver_is_generic: true, // <-- the key: call site had generics
        };

        // Should NOT resolve because the only List class is non-generic
        let resolved = resolve_call_site(&call_generic, &def_idx, None);
        assert!(resolved.is_empty(),
            "Generic call new List<CatalogEntry>() should NOT resolve to non-generic List class, got {:?}", resolved);

        // Call site: new List() — non-generic call
        let call_non_generic = CallSite {
            method_name: "List".to_string(),
            receiver_type: Some("List".to_string()),
            line: 300,
            receiver_is_generic: false,
        };

        // SHOULD resolve — both non-generic
        let resolved2 = resolve_call_site(&call_non_generic, &def_idx, None);
        assert!(!resolved2.is_empty(),
            "Non-generic call new List() SHOULD resolve to non-generic List class");
    }
}