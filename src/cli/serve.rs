//! MCP server startup and configuration.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use tracing::{info, warn};

use crate::{
    build_content_index, save_content_index, load_content_index, find_content_index_for_dir,
    index_dir, DEFAULT_MIN_TOKEN_LEN,
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

    // Load or build content index
    let start = Instant::now();
    let index = match load_content_index(&dir_str, &exts_for_load, &idx_base) {
        Some(idx) => {
            info!(files = idx.files.len(), tokens = idx.index.len(), "Loaded content index");
            idx
        }
        None => {
            match find_content_index_for_dir(&dir_str, &idx_base) {
                Some(idx) => {
                    info!(files = idx.files.len(), tokens = idx.index.len(), "Found content index for directory");
                    idx
                }
                None => {
                    info!("No content index found, building from scratch");
                    let new_idx = build_content_index(&ContentIndexArgs {
                        dir: dir_str.clone(),
                        ext: exts_for_load.clone(),
                        max_age_hours: 24,
                        hidden: false,
                        no_ignore: false,
                        threads: 0,
                        min_token_len: DEFAULT_MIN_TOKEN_LEN,
                    });
                    if let Err(e) = save_content_index(&new_idx, &idx_base) {
                        warn!(error = %e, "Failed to save content index to disk");
                    }
                    new_idx
                }
            }
        }
    };

    let load_elapsed = start.elapsed();
    info!(
        elapsed_ms = format_args!("{:.1}", load_elapsed.as_secs_f64() * 1000.0),
        files = index.files.len(),
        tokens = index.index.len(),
        "Content index ready"
    );

    let index = if args.watch {
        let watch_idx = mcp::watcher::build_watch_index_from(index);
        Arc::new(RwLock::new(watch_idx))
    } else {
        Arc::new(RwLock::new(index))
    };

    // Load or build definition index if --definitions
    let def_index = if args.definitions {
        let def_start = Instant::now();
        // Supported definition languages (no SQL — currently unsupported)
        let supported_def_langs: &[&str] = &["cs", "ts", "tsx"];
        // Intersect with user-provided --ext so we only parse languages actually requested
        let def_exts = supported_def_langs.iter()
            .filter(|lang| extensions.iter().any(|e| e.eq_ignore_ascii_case(lang)))
            .copied()
            .collect::<Vec<&str>>()
            .join(",");
        // Fallback: if no intersection (e.g. user passed only xml,config), use "cs" as minimum
        let def_exts = if def_exts.is_empty() { "cs".to_string() } else { def_exts };

        let def_idx = match definitions::load_definition_index(&dir_str, &def_exts, &idx_base) {
            Some(idx) => {
                info!(definitions = idx.definitions.len(), files = idx.files.len(),
                    "Loaded definition index from disk");
                idx
            }
            None => {
                match definitions::find_definition_index_for_dir(&dir_str, &idx_base) {
                    Some(idx) => {
                        info!(definitions = idx.definitions.len(), files = idx.files.len(),
                            "Found definition index for directory");
                        idx
                    }
                    None => {
                        info!("No definition index found, building with tree-sitter AST parsing");
                        let new_idx = definitions::build_definition_index(&definitions::DefIndexArgs {
                            dir: dir_str.clone(),
                            ext: def_exts.to_string(),
                            threads: 0,
                        });
                        if let Err(e) = definitions::save_definition_index(&new_idx, &idx_base) {
                            warn!(error = %e, "Failed to save definition index to disk");
                        }
                        new_idx
                    }
                }
            }
        };

        let def_elapsed = def_start.elapsed();
        info!(
            elapsed_ms = format_args!("{:.1}", def_elapsed.as_secs_f64() * 1000.0),
            definitions = def_idx.definitions.len(),
            files = def_idx.files.len(),
            "Definition index ready"
        );
        Some(Arc::new(RwLock::new(def_idx)))
    } else {
        None
    };

    // Start file watcher if --watch
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
    mcp::server::run_server(index, def_index, dir_str, exts_for_load, args.metrics, idx_base, max_response_bytes);
}