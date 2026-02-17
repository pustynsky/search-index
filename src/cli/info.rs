//! info and info_json commands.

use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{index_dir, ContentIndex, FileIndex};

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
            if let Ok(data) = fs::read(&path)
                && let Ok(index) = bincode::deserialize::<FileIndex>(&data) {
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
        } else if ext == Some("cidx")
            && let Ok(data) = fs::read(&path)
                && let Ok(index) = bincode::deserialize::<ContentIndex>(&data) {
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
                if let Ok(data) = fs::read(&path)
                    && let Ok(index) = bincode::deserialize::<FileIndex>(&data) {
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
            } else if ext == Some("cidx")
                && let Ok(data) = fs::read(&path)
                    && let Ok(index) = bincode::deserialize::<ContentIndex>(&data) {
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
            else if ext == Some("didx")
                && let Ok(data) = fs::read(&path)
                    && let Ok(index) = bincode::deserialize::<crate::definitions::DefinitionIndex>(&data) {
                        let age_secs = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or(Duration::ZERO)
                            .as_secs()
                            .saturating_sub(index.created_at);
                        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        let call_sites: usize = index.method_calls.values().map(|v| v.len()).sum();
                        indexes.push(serde_json::json!({
                            "type": "definition",
                            "root": index.root,
                            "files": index.files.len(),
                            "definitions": index.definitions.len(),
                            "callSites": call_sites,
                            "extensions": index.extensions,
                            "sizeMb": (size as f64 / 1_048_576.0 * 10.0).round() / 10.0,
                            "ageHours": (age_secs as f64 / 3600.0 * 10.0).round() / 10.0,
                        }));
                    }
        }
    }

    serde_json::json!({
        "directory": dir.display().to_string(),
        "indexes": indexes,
    })
}