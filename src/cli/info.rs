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

        let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("?");

        if ext == Some("file-list") {
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
                        "  [FILE] {} -- {} entries, {:.1} MB, {:.1}h ago{} ({})",
                        index.root, index.entries.len(),
                        size as f64 / 1_048_576.0, age_hours, stale, filename
                    );
                }
                Err(e) => {
                    eprintln!("  Warning: failed to load {}: {}", path.display(), e);
                }
            }
        } else if ext == Some("word-search") {
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
                        "  [CONTENT] {} -- {} files, {} tokens, exts: [{}], {:.1} MB, {:.1}h ago{} ({})",
                        index.root, index.files.len(), index.total_tokens,
                        index.extensions.join(", "),
                        size as f64 / 1_048_576.0, age_hours, stale, filename
                    );
                }
                Err(e) => {
                    eprintln!("  Warning: failed to load {}: {}", path.display(), e);
                }
            }
        } else if ext == Some("git-history") {
            if let Ok(cache) = crate::git::cache::GitHistoryCache::load_from_disk(&path) {
                found = true;
                let age_secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO)
                    .as_secs()
                    .saturating_sub(cache.built_at);
                let age_hours = age_secs as f64 / 3600.0;
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                println!(
                    "  [GIT] branch={}, {} commits, {} files, {} authors, HEAD={}, {:.1} MB, {:.1}h ago ({})",
                    cache.branch,
                    cache.commits.len(),
                    cache.file_commits.len(),
                    cache.authors.len(),
                    &cache.head_hash[..cache.head_hash.len().min(8)],
                    size as f64 / 1_048_576.0,
                    age_hours,
                    filename
                );
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

            let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("?").to_string();

            if ext == Some("file-list") {
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
                        "filename": filename,
                    }));
                }
            } else if ext == Some("word-search") {
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
                        "filename": filename,
                    }));
                }
            } else if ext == Some("code-structure") {
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
                    def_info["filename"] = serde_json::json!(filename);
                    indexes.push(def_info);
                }
            } else if ext == Some("git-history") {
                if let Ok(cache) = crate::git::cache::GitHistoryCache::load_from_disk(&path) {
                    let age_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or(Duration::ZERO)
                        .as_secs()
                        .saturating_sub(cache.built_at);
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    indexes.push(serde_json::json!({
                        "type": "git-history",
                        "commits": cache.commits.len(),
                        "files": cache.file_commits.len(),
                        "authors": cache.authors.len(),
                        "headHash": cache.head_hash,
                        "branch": cache.branch,
                        "sizeMb": (size as f64 / 1_048_576.0 * 10.0).round() / 10.0,
                        "ageHours": (age_secs as f64 / 3600.0 * 10.0).round() / 10.0,
                        "filename": filename,
                    }));
                }
            }
        }
    }

    serde_json::json!({
        "directory": dir.display().to_string(),
        "indexes": indexes,
    })
}

/// Return index info as JSON value for a specific directory (for testing).
/// This is like `cmd_info_json` but operates on a given directory path.
#[cfg(test)]
pub(crate) fn cmd_info_json_for_dir(dir: &std::path::Path) -> serde_json::Value {
    if !dir.exists() {
        return serde_json::json!({ "indexes": [], "directory": dir.display().to_string() });
    }

    let mut indexes = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());

            let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("?").to_string();

            if ext == Some("file-list") {
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
                        "filename": filename,
                    }));
                }
            } else if ext == Some("word-search") {
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
                        "filename": filename,
                    }));
                }
            } else if ext == Some("code-structure") {
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
                    def_info["filename"] = serde_json::json!(filename);
                    indexes.push(def_info);
                }
            } else if ext == Some("git-history") {
                if let Ok(cache) = crate::git::cache::GitHistoryCache::load_from_disk(&path) {
                    let age_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or(Duration::ZERO)
                        .as_secs()
                        .saturating_sub(cache.built_at);
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    indexes.push(serde_json::json!({
                        "type": "git-history",
                        "commits": cache.commits.len(),
                        "files": cache.file_commits.len(),
                        "authors": cache.authors.len(),
                        "headHash": cache.head_hash,
                        "branch": cache.branch,
                        "sizeMb": (size as f64 / 1_048_576.0 * 10.0).round() / 10.0,
                        "ageHours": (age_secs as f64 / 3600.0 * 10.0).round() / 10.0,
                        "filename": filename,
                    }));
                }
            }
        }
    }

    serde_json::json!({
        "directory": dir.display().to_string(),
        "indexes": indexes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal GitHistoryCache with test data and save it to a temp directory.
    fn create_test_git_history_cache(dir: &std::path::Path) -> std::path::PathBuf {
        use crate::git::cache::{GitHistoryCacheBuilder, parse_git_log_stream};

        // Use the public streaming parser to build a cache
        let git_log = "\
COMMIT:aabbccddee00112233445566778899aabbccddee␞1700000000␞alice@example.com␞Alice␞Initial commit
src/main.rs

COMMIT:112233445566778899aabbccddeeff0011223344␞1700001000␞bob@example.com␞Bob␞Add feature
src/main.rs
src/lib.rs
";
        let mut builder = GitHistoryCacheBuilder::new();
        let reader = std::io::BufReader::new(git_log.as_bytes());
        parse_git_log_stream(reader, &mut builder).unwrap();

        let cache = builder.build(
            "aabbccddee00112233445566778899aabbccddee".to_string(),
            "main".to_string(),
        );

        let cache_path = dir.join("test_12345678.git-history");
        cache.save_to_disk(&cache_path).unwrap();
        cache_path
    }

    #[test]
    fn test_info_json_includes_git_history() {
        let tmp = std::env::temp_dir().join(format!("search_info_test_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let _cache_path = create_test_git_history_cache(&tmp);

        let result = cmd_info_json_for_dir(&tmp);

        let indexes = result["indexes"].as_array().expect("indexes should be an array");
        let git_entries: Vec<_> = indexes.iter()
            .filter(|idx| idx["type"] == "git-history")
            .collect();

        assert_eq!(git_entries.len(), 1, "Expected exactly 1 git-history entry");

        let entry = &git_entries[0];
        assert_eq!(entry["type"], "git-history");
        assert_eq!(entry["commits"], 2);
        assert_eq!(entry["files"], 2); // src/main.rs and src/lib.rs
        assert_eq!(entry["authors"], 2); // Alice and Bob
        assert_eq!(entry["branch"], "main");
        assert_eq!(entry["headHash"], "aabbccddee00112233445566778899aabbccddee");
        assert!(entry["sizeMb"].as_f64().unwrap() >= 0.0);
        assert!(entry["ageHours"].as_f64().unwrap() >= 0.0);
        assert!(entry["filename"].as_str().unwrap().ends_with(".git-history"));

        // Cleanup
        std::fs::remove_dir_all(&tmp).unwrap_or_default();
    }

    #[test]
    fn test_info_json_empty_dir_no_git_history() {
        let tmp = std::env::temp_dir().join(format!("search_info_empty_test_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let result = cmd_info_json_for_dir(&tmp);
        let indexes = result["indexes"].as_array().expect("indexes should be an array");
        let git_entries: Vec<_> = indexes.iter()
            .filter(|idx| idx["type"] == "git-history")
            .collect();
        assert_eq!(git_entries.len(), 0, "Empty dir should have no git-history entries");

        // Cleanup
        std::fs::remove_dir_all(&tmp).unwrap_or_default();
    }

    #[test]
    fn test_info_json_nonexistent_dir() {
        let nonexistent = std::path::Path::new("/nonexistent_search_info_test_dir_12345");
        let result = cmd_info_json_for_dir(nonexistent);
        let indexes = result["indexes"].as_array().expect("indexes should be an array");
        assert!(indexes.is_empty());
    }

    #[test]
    fn test_info_json_git_history_corrupt_file_skipped() {
        let tmp = std::env::temp_dir().join(format!("search_info_corrupt_test_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        // Write a corrupt .git-history file
        let corrupt_path = tmp.join("corrupt_12345678.git-history");
        std::fs::write(&corrupt_path, b"THIS_IS_NOT_A_VALID_GIT_CACHE").unwrap();

        let result = cmd_info_json_for_dir(&tmp);
        let indexes = result["indexes"].as_array().expect("indexes should be an array");
        let git_entries: Vec<_> = indexes.iter()
            .filter(|idx| idx["type"] == "git-history")
            .collect();
        assert_eq!(git_entries.len(), 0, "Corrupt git-history file should be skipped");

        // Cleanup
        std::fs::remove_dir_all(&tmp).unwrap_or_default();
    }
}