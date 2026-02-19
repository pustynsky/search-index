//! MCP server startup and configuration.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use tracing::{info, warn};

use crate::{
    build_content_index, save_content_index, load_content_index, find_content_index_for_dir,
    index_dir, ContentIndex, TrigramIndex, DEFAULT_MIN_TOKEN_LEN,
};
use crate::definitions;
use crate::mcp;

use super::args::{ServeArgs, ContentIndexArgs};

pub fn cmd_serve(args: ServeArgs) {
    let dir_str = args.dir.clone();
    let ext_str = args.ext.clone();
    let extensions: Vec<String> = ext_str.split(',').map(|s| s.trim().to_lowercase()).collect();
    let exts_for_load = extensions.join(",");

    let log_level = match args.log_level.as_str() {
        "error" => tracing::Level::ERROR,
        "warn" => tracing::Level::WARN,
        "debug" => tracing::Level::DEBUG,
        "trace" => tracing::Level::TRACE,
        _ => tracing::Level::INFO,
    };
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(true)
        .with_writer(std::io::stderr)
        .init();

    info!(dir = %dir_str, ext = %exts_for_load, "Starting MCP server");

    let idx_base = index_dir();

    // ─── Async startup: create empty indexes, start event loop immediately ───
    use std::collections::HashMap;

    let content_ready = Arc::new(AtomicBool::new(false));
    let def_ready = Arc::new(AtomicBool::new(false));

    // Create an empty ContentIndex so the event loop can start immediately
    let empty_index = ContentIndex {
        root: dir_str.clone(),
        created_at: 0,
        max_age_secs: 86400,
        files: Vec::new(),
        index: HashMap::new(),
        total_tokens: 0,
        extensions: extensions.clone(),
        file_token_counts: Vec::new(),
        trigram: TrigramIndex::default(),
        trigram_dirty: false,
        forward: None,  // forward index eliminated — saves ~1.5 GB RAM
        path_to_id: if args.watch { Some(HashMap::new()) } else { None },
    };
    let index = Arc::new(RwLock::new(empty_index));

    // Try fast load from disk (typically < 3s)
    let start = Instant::now();
    let loaded = load_content_index(&dir_str, &exts_for_load, &idx_base)
        .ok()
        .or_else(|| find_content_index_for_dir(&dir_str, &idx_base));

    if let Some(idx) = loaded {
        let load_elapsed = start.elapsed();
        info!(
            elapsed_ms = format_args!("{:.1}", load_elapsed.as_secs_f64() * 1000.0),
            files = idx.files.len(),
            tokens = idx.index.len(),
            "Content index loaded from disk"
        );
        let idx = if args.watch {
            mcp::watcher::build_watch_index_from(idx)
        } else {
            idx
        };
        *index.write().unwrap() = idx;
        content_ready.store(true, Ordering::Release);
    } else {
        // Build in background — don't block the event loop
        let bg_index: Arc<RwLock<ContentIndex>> = Arc::clone(&index);
        let bg_ready = Arc::clone(&content_ready);
        let bg_dir = dir_str.clone();
        let bg_ext = exts_for_load.clone();
        let bg_idx_base = idx_base.clone();
        let bg_watch = args.watch;

        std::thread::spawn(move || {
            info!("Building content index in background...");
            let build_start = Instant::now();
            let new_idx = build_content_index(&ContentIndexArgs {
                dir: bg_dir.clone(),
                ext: bg_ext.clone(),
                max_age_hours: 24,
                hidden: false,
                no_ignore: false,
                threads: 0,
                min_token_len: DEFAULT_MIN_TOKEN_LEN,
            });
            if let Err(e) = save_content_index(&new_idx, &bg_idx_base) {
                warn!(error = %e, "Failed to save content index to disk");
            }

            // Drop build-time index and reload from disk to eliminate allocator
            // fragmentation (~1.5 GB savings). Build creates many temporary allocs
            // that fragment the heap; reloading gives compact contiguous memory.
            let file_count = new_idx.files.len();
            let token_count = new_idx.index.len();
            drop(new_idx);
            let new_idx = load_content_index(&bg_dir, &bg_ext, &bg_idx_base)
                .unwrap_or_else(|e| {
                    warn!(error = %e, "Failed to reload content index from disk, rebuilding");
                    build_content_index(&ContentIndexArgs {
                        dir: bg_dir, ext: bg_ext,
                        max_age_hours: 24, hidden: false, no_ignore: false,
                        threads: 0, min_token_len: DEFAULT_MIN_TOKEN_LEN,
                    })
                });

            let new_idx = if bg_watch {
                mcp::watcher::build_watch_index_from(new_idx)
            } else {
                new_idx
            };
            let elapsed = build_start.elapsed();
            info!(
                elapsed_ms = format_args!("{:.1}", elapsed.as_secs_f64() * 1000.0),
                files = file_count,
                tokens = token_count,
                "Content index ready (background build complete)"
            );
            *bg_index.write().unwrap() = new_idx;
            bg_ready.store(true, Ordering::Release);
        });
    }

    // ─── Definition index: same async pattern ───
    // Supported definition languages (no SQL — currently unsupported)
    let supported_def_langs: &[&str] = &["cs", "ts", "tsx"];
    let def_exts = supported_def_langs.iter()
        .filter(|lang| extensions.iter().any(|e| e.eq_ignore_ascii_case(lang)))
        .copied()
        .collect::<Vec<&str>>()
        .join(",");
    let def_exts = if def_exts.is_empty() { "cs".to_string() } else { def_exts };

    let def_index = if args.definitions {
        // Create an empty DefinitionIndex placeholder
        let empty_def = definitions::DefinitionIndex {
            root: dir_str.clone(),
            created_at: 0,
            extensions: def_exts.split(',').map(|s| s.to_string()).collect(),
            files: Vec::new(),
            definitions: Vec::new(),
            name_index: HashMap::new(),
            kind_index: HashMap::new(),
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index: HashMap::new(),
            path_to_id: HashMap::new(),
            method_calls: HashMap::new(),
            parse_errors: 0,
            lossy_file_count: 0,
            empty_file_ids: Vec::new(),
        };
        let def_arc = Arc::new(RwLock::new(empty_def));

        // Try fast load from disk
        let def_start = Instant::now();
        let def_loaded = definitions::load_definition_index(&dir_str, &def_exts, &idx_base)
            .ok()
            .or_else(|| definitions::find_definition_index_for_dir(&dir_str, &idx_base));

        if let Some(idx) = def_loaded {
            let def_elapsed = def_start.elapsed();
            info!(
                elapsed_ms = format_args!("{:.1}", def_elapsed.as_secs_f64() * 1000.0),
                definitions = idx.definitions.len(),
                files = idx.files.len(),
                "Definition index loaded from disk"
            );
            *def_arc.write().unwrap() = idx;
            def_ready.store(true, Ordering::Release);
        } else {
            // Build in background
            let bg_def = Arc::clone(&def_arc);
            let bg_def_ready = Arc::clone(&def_ready);
            let bg_dir = dir_str.clone();
            let bg_def_exts = def_exts.clone();
            let bg_idx_base = idx_base.clone();

            std::thread::spawn(move || {
                info!("Building definition index in background...");
                let build_start = Instant::now();
                let new_idx = definitions::build_definition_index(&definitions::DefIndexArgs {
                    dir: bg_dir.clone(),
                    ext: bg_def_exts.clone(),
                    threads: 0,
                });
                if let Err(e) = definitions::save_definition_index(&new_idx, &bg_idx_base) {
                    warn!(error = %e, "Failed to save definition index to disk");
                }

                // Drop + reload to eliminate allocator fragmentation (same pattern)
                let def_count = new_idx.definitions.len();
                let file_count = new_idx.files.len();
                drop(new_idx);
                let new_idx = definitions::load_definition_index(&bg_dir, &bg_def_exts, &bg_idx_base)
                    .unwrap_or_else(|e| {
                        warn!(error = %e, "Failed to reload definition index from disk, rebuilding");
                        definitions::build_definition_index(&definitions::DefIndexArgs {
                            dir: bg_dir, ext: bg_def_exts, threads: 0,
                        })
                    });

                let elapsed = build_start.elapsed();
                info!(
                    elapsed_ms = format_args!("{:.1}", elapsed.as_secs_f64() * 1000.0),
                    definitions = def_count,
                    files = file_count,
                    "Definition index ready (background build complete)"
                );
                *bg_def.write().unwrap() = new_idx;
                bg_def_ready.store(true, Ordering::Release);
            });
        }

        Some(def_arc)
    } else {
        // No --definitions flag: mark as ready (N/A)
        def_ready.store(true, Ordering::Release);
        None
    };

    // Start file watcher if --watch (only after content index is available)
    // Watcher works fine with an empty index — it will update it as files change.
    if args.watch {
        let watch_dir = std::fs::canonicalize(&dir_str)
            .unwrap_or_else(|_| PathBuf::from(&dir_str));
        if let Err(e) = mcp::watcher::start_watcher(
            Arc::clone(&index),
            def_index.as_ref().map(Arc::clone),
            watch_dir,
            extensions,
            args.debounce_ms,
            args.bulk_threshold,
            idx_base.clone(),
        ) {
            warn!(error = %e, "Failed to start file watcher");
        }
    }

    let max_response_bytes = if args.max_response_kb == 0 { 0 } else { args.max_response_kb * 1024 };
    mcp::server::run_server(
        index, def_index, dir_str, exts_for_load,
        args.metrics, idx_base, max_response_bytes,
        content_ready, def_ready,
    );
}