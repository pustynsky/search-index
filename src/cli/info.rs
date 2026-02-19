//! info and info_json commands.

use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{index_dir, index::load_compressed, ContentIndex, FileIndex};

pub fn cmd_info() {
    let dir = index_dir();
    if !dir.exists() {
        eprintln!("No indexes found. Use 'search index -d <dir>' to create one.");
        return;
    }

    eprintln!("Index directory: {}", dir.display());
    eprintln!();

    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to read index directory: {}", e);
            return;
        }
    };

    let mut found = false;
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str());

        if ext == Some("idx") {
            match load_compressed::<FileIndex>(&path, "file-index") {
                Ok(index) => {
                    found = true;
                    let age_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or(Duration::ZERO)
                        .as_secs()
                        .saturating_sub(index.created_at);
                    let age_hours = age_secs as f64 / 3600.0;
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let stale = if index.is_stale() { " [STALE]" } else { "" };
                    println!(
                        "  [FILE] {} -- {} entries, {:.1} MB, {:.1}h ago{}",
                        index.root, index.entries.len(),
                        size as f64 / 1_048_576.0, age_hours, stale
                    );
                }
                Err(e) => {
                    eprintln!("  Warning: failed to load {}: {}", path.display(), e);
                }
            }
        } else if ext == Some("cidx") {
            match load_compressed::<ContentIndex>(&path, "content-index") {
                Ok(index) => {
                    found = true;
                    let age_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or(Duration::ZERO)
                        .as_secs()
                        .saturating_sub(index.created_at);
                    let age_hours = age_secs as f64 / 3600.0;
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let stale = if index.is_stale() { " [STALE]" } else { "" };
                    println!(
                        "  [CONTENT] {} -- {} files, {} tokens, exts: [{}], {:.1} MB, {:.1}h ago{}",
                        index.root, index.files.len(), index.total_tokens,
                        index.extensions.join(", "),
                        size as f64 / 1_048_576.0, age_hours, stale
                    );
                }
                Err(e) => {
                    eprintln!("  Warning: failed to load {}: {}", path.display(), e);
                }
            }
        }
    }

    if !found {
        eprintln!("No indexes found.");
    }
}

/// Return index info as JSON value (for MCP handler)
pub fn cmd_info_json() -> serde_json::Value {
    let dir = index_dir();
    if !dir.exists() {
        return serde_json::json!({ "indexes": [], "directory": dir.display().to_string() });
    }

    let mut indexes = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());

            if ext == Some("idx") {
                if let Ok(index) = load_compressed::<FileIndex>(&path, "file-index") {
                    let age_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or(Duration::ZERO)
                        .as_secs()
                        .saturating_sub(index.created_at);
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    indexes.push(serde_json::json!({
                        "type": "file",
                        "root": index.root,
                        "entries": index.entries.len(),
                        "sizeMb": (size as f64 / 1_048_576.0 * 10.0).round() / 10.0,
                        "ageHours": (age_secs as f64 / 3600.0 * 10.0).round() / 10.0,
                        "stale": index.is_stale(),
                    }));
                }
            } else if ext == Some("cidx") {
                if let Ok(index) = load_compressed::<ContentIndex>(&path, "content-index") {
                    let age_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or(Duration::ZERO)
                        .as_secs()
                        .saturating_sub(index.created_at);
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    indexes.push(serde_json::json!({
                        "type": "content",
                        "root": index.root,
                        "files": index.files.len(),
                        "totalTokens": index.total_tokens,
                        "extensions": index.extensions,
                        "sizeMb": (size as f64 / 1_048_576.0 * 10.0).round() / 10.0,
                        "ageHours": (age_secs as f64 / 3600.0 * 10.0).round() / 10.0,
                        "stale": index.is_stale(),
                    }));
                }
            } else if ext == Some("didx") {
                if let Ok(index) = load_compressed::<crate::definitions::DefinitionIndex>(&path, "definition-index") {
                    let age_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or(Duration::ZERO)
                        .as_secs()
                        .saturating_sub(index.created_at);
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let call_sites: usize = index.method_calls.values().map(|v| v.len()).sum();
                    let mut def_info = serde_json::json!({
                        "type": "definition",
                        "root": index.root,
                        "files": index.files.len(),
                        "definitions": index.definitions.len(),
                        "callSites": call_sites,
                        "extensions": index.extensions,
                        "sizeMb": (size as f64 / 1_048_576.0 * 10.0).round() / 10.0,
                        "ageHours": (age_secs as f64 / 3600.0 * 10.0).round() / 10.0,
                    });
                    if index.parse_errors > 0 {
                        def_info["readErrors"] = serde_json::json!(index.parse_errors);
                    }
                    if index.lossy_file_count > 0 {
                        def_info["lossyUtf8Files"] = serde_json::json!(index.lossy_file_count);
                    }
                    indexes.push(def_info);
                }
            }
        }
    }

    serde_json::json!({
        "directory": dir.display().to_string(),
        "indexes": indexes,
    })
}