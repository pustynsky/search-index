//! Definition index: AST-based code structure extraction using tree-sitter.

mod types;
mod parser_csharp;
mod parser_typescript;
mod parser_sql;
mod storage;
mod incremental;

// Re-export all public types and functions
pub use types::*;
pub use storage::*;
pub use incremental::*;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;

use crate::{clean_path, read_file_lossy};

// ─── Index Build ─────────────────────────────────────────────────────

pub fn build_definition_index(args: &DefIndexArgs) -> DefinitionIndex {
    let dir = std::fs::canonicalize(&args.dir)
        .unwrap_or_else(|_| PathBuf::from(&args.dir));
    let dir_str = clean_path(&dir.to_string_lossy());

    let extensions: Vec<String> = args.ext.split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let start = Instant::now();

    // Collect all files
    let mut walker = WalkBuilder::new(&dir);
    walker.hidden(false).git_ignore(true);
    if args.threads > 0 {
        walker.threads(args.threads);
    }

    let file_count = AtomicUsize::new(0);
    let all_files: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    walker.build_parallel().run(|| {
        Box::new(|entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return ignore::WalkState::Continue,
            };
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                return ignore::WalkState::Continue;
            }
            let path = entry.path();
            let ext_match = path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)));
            if !ext_match {
                return ignore::WalkState::Continue;
            }
            let clean = clean_path(&path.to_string_lossy());
            all_files.lock().unwrap_or_else(|e| e.into_inner()).push(clean);
            file_count.fetch_add(1, Ordering::Relaxed);
            ignore::WalkState::Continue
        })
    });

    let files: Vec<String> = all_files.into_inner().unwrap();
    let total_files = files.len();
    eprintln!("[def-index] Found {} files to parse", total_files);

    // SQL grammar is currently disabled
    let sql_available = false;

    // ─── Parallel parsing ─────────────────────────────────────
    let num_threads = if args.threads > 0 {
        args.threads
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    };
    let chunk_size = total_files.div_ceil(num_threads);
    let chunks: Vec<Vec<(u32, String)>> = files.iter().enumerate()
        .map(|(i, f)| (i as u32, f.clone()))
        .collect::<Vec<_>>()
        .chunks(chunk_size.max(1))
        .map(|c| c.to_vec())
        .collect();

    eprintln!("[def-index] Parsing with {} threads ({} files/chunk)", chunks.len(), chunk_size);

    let sql_avail = sql_available;
    let thread_results: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = chunks.into_iter().map(|chunk| {
            s.spawn(move || {
                let mut cs_parser = tree_sitter::Parser::new();
                cs_parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into())
                    .expect("Error loading C# grammar");

                let mut ts_parser = tree_sitter::Parser::new();
                ts_parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
                    .expect("Error loading TypeScript grammar");

                let mut tsx_parser = tree_sitter::Parser::new();
                tsx_parser.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
                    .expect("Error loading TSX grammar");

                let mut sql_parser = tree_sitter::Parser::new();
                let _ = &sql_parser; // suppress unused warning

                let mut chunk_defs: Vec<(u32, Vec<DefinitionEntry>, Vec<(usize, Vec<CallSite>)>)> = Vec::new();
                let mut errors = 0usize;
                let mut lossy_files: Vec<String> = Vec::new();
                let mut empty_files: Vec<(u32, u64)> = Vec::new(); // (file_id, byte_size) for files with 0 defs

                for (file_id, file_path) in &chunk {
                    let (content, was_lossy) = match read_file_lossy(Path::new(file_path)) {
                        Ok(r) => r,
                        Err(_) => { errors += 1; continue; }
                    };
                    if was_lossy {
                        lossy_files.push(file_path.clone());
                    }

                    let content_len = content.len() as u64;

                    let ext = Path::new(file_path.as_str())
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");

                    let (file_defs, file_calls) = match ext.to_lowercase().as_str() {
                        "cs" => parser_csharp::parse_csharp_definitions(&mut cs_parser, &content, *file_id),
                        "ts" => parser_typescript::parse_typescript_definitions(&mut ts_parser, &content, *file_id),
                        "tsx" => parser_typescript::parse_typescript_definitions(&mut tsx_parser, &content, *file_id),
                        "sql" if sql_avail => (parser_sql::parse_sql_definitions(&mut sql_parser, &content, *file_id), Vec::new()),
                        _ => (Vec::new(), Vec::new()),
                    };

                    if !file_defs.is_empty() {
                        chunk_defs.push((*file_id, file_defs, file_calls));
                    } else {
                        empty_files.push((*file_id, content_len));
                    }
                }

                (chunk_defs, errors, lossy_files, empty_files)
            })
        }).collect();

        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // ─── Merge results ────────────────────────────────────────
    let mut definitions: Vec<DefinitionEntry> = Vec::new();
    let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
    let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
    let mut attribute_index: HashMap<String, Vec<u32>> = HashMap::new();
    let mut base_type_index: HashMap<String, Vec<u32>> = HashMap::new();
    let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();
    let mut method_calls: HashMap<u32, Vec<CallSite>> = HashMap::new();
    let mut parse_errors = 0usize;
    let mut total_call_sites = 0usize;

    // Build path_to_id from the files list
    for (file_id, file_path) in files.iter().enumerate() {
        path_to_id.insert(PathBuf::from(file_path), file_id as u32);
    }

    let mut lossy_file_count = 0usize;
    let mut empty_file_ids: Vec<(u32, u64)> = Vec::new();
    for (chunk_defs, errors, lossy_files, empty_files) in thread_results {
        parse_errors += errors;
        for f in &lossy_files {
            eprintln!("[def-index] WARNING: file contains non-UTF8 bytes (lossy conversion applied): {}", f);
        }
        lossy_file_count += lossy_files.len();
        empty_file_ids.extend(empty_files);
        for (file_id, file_defs, file_calls) in chunk_defs {
            let base_def_idx = definitions.len() as u32;

            for def in file_defs {
                let def_idx = definitions.len() as u32;

                name_index.entry(def.name.to_lowercase())
                    .or_default()
                    .push(def_idx);

                kind_index.entry(def.kind.clone())
                    .or_default()
                    .push(def_idx);

                {
                    let mut seen_attrs = std::collections::HashSet::new();
                    for attr in &def.attributes {
                        let attr_name = attr.split('(').next().unwrap_or(attr).trim().to_lowercase();
                        if seen_attrs.insert(attr_name.clone()) {
                            attribute_index.entry(attr_name)
                                .or_default()
                                .push(def_idx);
                        }
                    }
                }

                for bt in &def.base_types {
                    base_type_index.entry(bt.to_lowercase())
                        .or_default()
                        .push(def_idx);
                }

                file_index.entry(file_id)
                    .or_default()
                    .push(def_idx);

                definitions.push(def);
            }

            // Map local call site indices to global def indices
            for (local_idx, calls) in file_calls {
                let global_idx = base_def_idx + local_idx as u32;
                if !calls.is_empty() {
                    total_call_sites += calls.len();
                    method_calls.insert(global_idx, calls);
                }
            }
        }
    }

    // Report suspicious files (>500 bytes but 0 definitions)
    let suspicious_threshold = 500u64;
    let suspicious: Vec<_> = empty_file_ids.iter()
        .filter(|(_, size)| *size > suspicious_threshold)
        .collect();
    if !suspicious.is_empty() {
        eprintln!("[def-index] WARNING: {} files with >{}B but 0 definitions. Run 'search def-audit' to see full list.",
            suspicious.len(), suspicious_threshold);
    }

    let elapsed = start.elapsed();
    let files_with_defs = total_files - empty_file_ids.len() - parse_errors;
    eprintln!(
        "[def-index] Parsed {} files in {:.1}s: {} with definitions, {} empty, {} read errors, {} lossy-utf8, {} threads",
        total_files,
        elapsed.as_secs_f64(),
        files_with_defs,
        empty_file_ids.len(),
        parse_errors,
        lossy_file_count,
        num_threads
    );
    eprintln!(
        "[def-index] Extracted {} definitions, {} call sites",
        definitions.len(),
        total_call_sites,
    );

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();

    DefinitionIndex {
        root: dir_str,
        created_at: now,
        extensions,
        files,
        definitions,
        name_index,
        kind_index,
        attribute_index,
        base_type_index,
        file_index,
        path_to_id,
        method_calls,
        parse_errors,
        lossy_file_count,
        empty_file_ids,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "definitions_tests.rs"]
mod tests;