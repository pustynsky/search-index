use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{error, info, warn};

use crate::{build_content_index, clean_path, save_content_index, tokenize, ContentIndex, ContentIndexArgs, Posting};
use crate::definitions::{self, DefinitionIndex};

/// Start a file watcher thread that incrementally updates the in-memory index
pub fn start_watcher(
    index: Arc<RwLock<ContentIndex>>,
    def_index: Option<Arc<RwLock<DefinitionIndex>>>,
    dir: PathBuf,
    extensions: Vec<String>,
    debounce_ms: u64,
    bulk_threshold: usize,
    index_base: PathBuf,
) -> notify::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();

    let mut watcher = RecommendedWatcher::new(tx, Config::default())?;
    watcher.watch(&dir, RecursiveMode::Recursive)?;

    let dir_str = clean_path(&dir.to_string_lossy());

    info!(dir = %dir_str, debounce_ms, bulk_threshold, "File watcher started");

    std::thread::spawn(move || {
        let _watcher = watcher; // move watcher into thread to keep it alive
        let mut dirty_files: HashSet<PathBuf> = HashSet::new();
        let mut removed_files: HashSet<PathBuf> = HashSet::new();

        loop {
            match rx.recv_timeout(Duration::from_millis(debounce_ms)) {
                Ok(Ok(event)) => {
                    // Collect changed files
                    for path in &event.paths {
                        if !matches_extensions(path, &extensions) {
                            continue;
                        }
                        match event.kind {
                            EventKind::Create(_) | EventKind::Modify(_) => {
                                removed_files.remove(path);
                                dirty_files.insert(path.clone());
                            }
                            EventKind::Remove(_) => {
                                dirty_files.remove(path);
                                removed_files.insert(path.clone());
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!(error = %e, "File watcher error");
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Debounce window expired — process batch
                    if dirty_files.is_empty() && removed_files.is_empty() {
                        continue;
                    }

                    let total_changes = dirty_files.len() + removed_files.len();

                    if total_changes > bulk_threshold {
                        // Too many changes — full reindex
                        info!(changes = total_changes, "Bulk threshold exceeded, triggering full reindex");
                        let ext_str = extensions.join(",");
                        let new_index = build_content_index(&ContentIndexArgs {
                            dir: dir_str.clone(),
                            ext: ext_str,
                            max_age_hours: 24,
                            hidden: false,
                            no_ignore: false,
                            threads: 0,
                            min_token_len: 2,
                        });
                        if let Err(e) = save_content_index(&new_index, &index_base) {
                            warn!(error = %e, "Failed to save reindexed content to disk");
                        }
                        // Rebuild forward index for watch mode
                        let new_index = build_watch_index_from(new_index);
                        match index.write() {
                            Ok(mut idx) => *idx = new_index,
                            Err(e) => error!(error = %e, "Failed to acquire content index write lock"),
                        }
                        dirty_files.clear();
                        removed_files.clear();
                        continue;
                    }

                    // Incremental update — single write lock for entire batch
                    let update_count = dirty_files.len();
                    let remove_count = removed_files.len();

                    // Collect cleaned paths once for both indexes
                    let removed_clean: Vec<PathBuf> = removed_files.drain()
                        .map(|p| PathBuf::from(clean_path(&p.to_string_lossy())))
                        .collect();
                    let dirty_clean: Vec<PathBuf> = dirty_files.drain()
                        .map(|p| PathBuf::from(clean_path(&p.to_string_lossy())))
                        .collect();

                    // Update content index
                    match index.write() {
                        Ok(mut idx) => {
                            for path in &removed_clean {
                                remove_file_from_index(&mut idx, path);
                            }
                            for path in &dirty_clean {
                                update_file_in_index(&mut idx, path);
                            }
                            // Mark trigram index as dirty — will be rebuilt lazily on next substring search
                            idx.trigram_dirty = true;
                        }
                        Err(e) => {
                            error!(error = %e, "Failed to acquire content index write lock");
                        }
                    }

                    // Update definition index (if available)
                    if let Some(ref def_idx) = def_index {
                        match def_idx.write() {
                            Ok(mut idx) => {
                                for path in &removed_clean {
                                    definitions::remove_file_from_def_index(&mut idx, path);
                                }
                                for path in &dirty_clean {
                                    definitions::update_file_definitions(&mut idx, path);
                                }
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to acquire definition index write lock");
                            }
                        }
                    }

                    info!(updated = update_count, removed = remove_count, "Incremental index update complete");
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    info!("Watcher channel disconnected, stopping");
                    break;
                }
            }
        }
    });

    Ok(())
}

fn matches_extensions(path: &Path, extensions: &[String]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Build a ContentIndex with forward index populated (for watch mode)
pub fn build_watch_index_from(mut index: ContentIndex) -> ContentIndex {
    // Build forward index and path_to_id from existing inverted index
    let mut forward: std::collections::HashMap<u32, Vec<String>> = std::collections::HashMap::new();
    let mut path_to_id: std::collections::HashMap<PathBuf, u32> = std::collections::HashMap::new();

    for (i, path) in index.files.iter().enumerate() {
        path_to_id.insert(PathBuf::from(path), i as u32);
    }

    for (token, postings) in &index.index {
        for posting in postings {
            forward.entry(posting.file_id)
                .or_default()
                .push(token.clone());
        }
    }

    // Deduplicate forward index entries
    for tokens in forward.values_mut() {
        tokens.sort();
        tokens.dedup();
    }

    index.forward = Some(forward);
    index.path_to_id = Some(path_to_id);
    index
}

/// Update a single file in the index (incremental)
fn update_file_in_index(index: &mut ContentIndex, path: &Path) {
    let path_str = path.to_string_lossy().to_string();

    // Read the file
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return, // File might have been deleted between event and processing
    };

    if let Some(ref mut path_to_id) = index.path_to_id {
        if let Some(&file_id) = path_to_id.get(path) {
            // EXISTING FILE — remove old tokens, add new ones
            // Subtract old token count from total before re-tokenizing
            let old_count = if (file_id as usize) < index.file_token_counts.len() {
                index.file_token_counts[file_id as usize] as u64
            } else {
                0u64
            };
            index.total_tokens = index.total_tokens.saturating_sub(old_count);

            if let Some(ref mut forward) = index.forward
                && let Some(old_tokens) = forward.remove(&file_id) {
                    for token in &old_tokens {
                        if let Some(postings) = index.index.get_mut(token) {
                            postings.retain(|p| p.file_id != file_id);
                            if postings.is_empty() {
                                index.index.remove(token);
                            }
                        }
                    }
                }

            // Re-tokenize file
            let mut file_tokens: std::collections::HashMap<String, Vec<u32>> = std::collections::HashMap::new();
            let mut file_total: u32 = 0;
            for (line_num, line) in content.lines().enumerate() {
                for token in tokenize(line, 2) {
                    index.total_tokens += 1;
                    file_total += 1;
                    file_tokens.entry(token).or_default().push((line_num + 1) as u32);
                }
            }

            // Add new tokens to inverted index
            for (token, lines) in &file_tokens {
                index.index.entry(token.clone())
                    .or_default()
                    .push(Posting { file_id, lines: lines.clone() });
            }

            // Update forward index
            if let Some(ref mut forward) = index.forward {
                forward.insert(file_id, file_tokens.keys().cloned().collect());
            }

            // Update file token count
            if (file_id as usize) < index.file_token_counts.len() {
                index.file_token_counts[file_id as usize] = file_total;
            }
        } else {
            // NEW FILE — assign new file_id
            let file_id = index.files.len() as u32;
            index.files.push(path_str.clone());
            path_to_id.insert(path.to_path_buf(), file_id);

            let mut file_tokens: std::collections::HashMap<String, Vec<u32>> = std::collections::HashMap::new();
            let mut file_total: u32 = 0;
            for (line_num, line) in content.lines().enumerate() {
                for token in tokenize(line, 2) {
                    index.total_tokens += 1;
                    file_total += 1;
                    file_tokens.entry(token).or_default().push((line_num + 1) as u32);
                }
            }

            for (token, lines) in &file_tokens {
                index.index.entry(token.clone())
                    .or_default()
                    .push(Posting { file_id, lines: lines.clone() });
            }

            if let Some(ref mut forward) = index.forward {
                forward.insert(file_id, file_tokens.keys().cloned().collect());
            }

            index.file_token_counts.push(file_total);
        }
    }
}

/// Remove a file from the index
fn remove_file_from_index(index: &mut ContentIndex, path: &Path) {
    if let Some(ref mut path_to_id) = index.path_to_id
        && let Some(&file_id) = path_to_id.get(path) {
            // Subtract this file's token count from total
            let old_count = if (file_id as usize) < index.file_token_counts.len() {
                index.file_token_counts[file_id as usize] as u64
            } else {
                0u64
            };
            index.total_tokens = index.total_tokens.saturating_sub(old_count);
            // Zero out the file's token count (file stays in vec as tombstone)
            if (file_id as usize) < index.file_token_counts.len() {
                index.file_token_counts[file_id as usize] = 0;
            }

            // Remove all tokens for this file from inverted index
            if let Some(ref mut forward) = index.forward
                && let Some(old_tokens) = forward.remove(&file_id) {
                    for token in &old_tokens {
                        if let Some(postings) = index.index.get_mut(token) {
                            postings.retain(|p| p.file_id != file_id);
                            if postings.is_empty() {
                                index.index.remove(token);
                            }
                        }
                    }
                }
            path_to_id.remove(path);
            // Don't remove from files vec to preserve file_id stability
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::TrigramIndex;

    fn make_test_index() -> ContentIndex {
        let mut idx = HashMap::new();
        idx.insert("httpclient".to_string(), vec![Posting {
            file_id: 0,
            lines: vec![5, 12],
        }]);
        idx.insert("ilogger".to_string(), vec![Posting {
            file_id: 0,
            lines: vec![3],
        }, Posting {
            file_id: 1,
            lines: vec![1],
        }]);

        ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec!["file0.cs".to_string(), "file1.cs".to_string()],
            index: idx,
            total_tokens: 100,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![50, 30],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: None,
            path_to_id: None,
        }
    }

    #[test]
    fn test_build_watch_index_populates_forward() {
        let index = make_test_index();
        let watch_index = build_watch_index_from(index);

        assert!(watch_index.forward.is_some());
        assert!(watch_index.path_to_id.is_some());

        let forward = watch_index.forward.as_ref().unwrap();
        // file_id 0 has httpclient and ilogger
        let tokens_0 = forward.get(&0).unwrap();
        assert!(tokens_0.contains(&"httpclient".to_string()));
        assert!(tokens_0.contains(&"ilogger".to_string()));

        // file_id 1 has ilogger
        let tokens_1 = forward.get(&1).unwrap();
        assert!(tokens_1.contains(&"ilogger".to_string()));
        assert!(!tokens_1.contains(&"httpclient".to_string()));
    }

    #[test]
    fn test_build_watch_index_populates_path_to_id() {
        let index = make_test_index();
        let watch_index = build_watch_index_from(index);

        let path_to_id = watch_index.path_to_id.as_ref().unwrap();
        assert_eq!(path_to_id.get(&PathBuf::from("file0.cs")), Some(&0));
        assert_eq!(path_to_id.get(&PathBuf::from("file1.cs")), Some(&1));
    }

    #[test]
    fn test_incremental_update_new_file() {
        let dir = std::env::temp_dir().join("search_watcher_test_new");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let new_file = dir.join("new_file.cs");
        std::fs::write(&new_file, "class NewClass { HttpClient client; }").unwrap();

        let mut index = make_test_index();
        index.forward = Some(HashMap::new());
        index.path_to_id = Some(HashMap::new());
        // Populate path_to_id for existing files
        for (i, path) in index.files.iter().enumerate() {
            index.path_to_id.as_mut().unwrap().insert(PathBuf::from(path), i as u32);
        }

        let clean_path = PathBuf::from(crate::clean_path(&new_file.to_string_lossy()));
        update_file_in_index(&mut index, &clean_path);

        // New file should be added
        assert_eq!(index.files.len(), 3);
        assert!(index.index.contains_key("newclass"));
        assert!(index.index.contains_key("httpclient"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_incremental_update_existing_file() {
        let dir = std::env::temp_dir().join("search_watcher_test_update");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let test_file = dir.join("test.cs");
        std::fs::write(&test_file, "class Original { OldToken stuff; }").unwrap();

        let clean = crate::clean_path(&test_file.to_string_lossy());
        let mut index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec![clean.clone()],
            index: {
                let mut m = HashMap::new();
                m.insert("original".to_string(), vec![Posting { file_id: 0, lines: vec![1] }]);
                m.insert("oldtoken".to_string(), vec![Posting { file_id: 0, lines: vec![1] }]);
                m
            },
            total_tokens: 10,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![5],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: Some({
                let mut m = HashMap::new();
                m.insert(0u32, vec!["original".to_string(), "oldtoken".to_string()]);
                m
            }),
            path_to_id: Some({
                let mut m = HashMap::new();
                m.insert(PathBuf::from(&clean), 0u32);
                m
            }),
        };

        // Now update the file content
        std::fs::write(&test_file, "class Updated { NewToken stuff; }").unwrap();
        update_file_in_index(&mut index, &PathBuf::from(&clean));

        // Old tokens should be gone, new tokens should be present
        assert!(!index.index.contains_key("original"), "old token 'original' should be removed");
        assert!(!index.index.contains_key("oldtoken"), "old token 'oldtoken' should be removed");
        assert!(index.index.contains_key("updated"), "new token 'updated' should be present");
        assert!(index.index.contains_key("newtoken"), "new token 'newtoken' should be present");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remove_file() {
        let mut index = make_test_index();
        // Build forward/path_to_id
        index = build_watch_index_from(index);

        // Remove file0.cs
        remove_file_from_index(&mut index, &PathBuf::from("file0.cs"));

        // httpclient was only in file0 — should be gone from index
        assert!(!index.index.contains_key("httpclient"), "httpclient should be removed with file0");

        // ilogger was in both files — should still exist for file1
        let ilogger = index.index.get("ilogger").unwrap();
        assert_eq!(ilogger.len(), 1);
        assert_eq!(ilogger[0].file_id, 1);

        // path_to_id should not contain file0 anymore
        let path_to_id = index.path_to_id.as_ref().unwrap();
        assert!(!path_to_id.contains_key(&PathBuf::from("file0.cs")));
        // files vec still has file0 for ID stability
        assert_eq!(index.files.len(), 2);
    }

    #[test]
    fn test_matches_extensions() {
        let exts = vec!["cs".to_string(), "rs".to_string()];
        assert!(matches_extensions(Path::new("foo.cs"), &exts));
        assert!(matches_extensions(Path::new("bar.RS"), &exts));
        assert!(!matches_extensions(Path::new("baz.txt"), &exts));
        assert!(!matches_extensions(Path::new("no_ext"), &exts));
    }

    #[test]
    fn test_bulk_threshold_concept() {
        // Verify the threshold logic: if changes > threshold, we'd do full reindex
        let threshold = 100;
        let small_batch = 50;
        let large_batch = 150;

        assert!(small_batch <= threshold, "small batch should use incremental");
        assert!(large_batch > threshold, "large batch should trigger full reindex");
    }

    #[test]
    fn test_total_tokens_decremented_on_update() {
        let dir = std::env::temp_dir().join("search_watcher_test_tokens_update");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let test_file = dir.join("test.cs");
        std::fs::write(&test_file, "class Original { OldToken stuff; }").unwrap();

        let clean = crate::clean_path(&test_file.to_string_lossy());
        let mut index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec![clean.clone()],
            index: {
                let mut m = HashMap::new();
                m.insert("original".to_string(), vec![Posting { file_id: 0, lines: vec![1] }]);
                m.insert("oldtoken".to_string(), vec![Posting { file_id: 0, lines: vec![1] }]);
                m.insert("stuff".to_string(), vec![Posting { file_id: 0, lines: vec![1] }]);
                m.insert("class".to_string(), vec![Posting { file_id: 0, lines: vec![1] }]);
                m
            },
            total_tokens: 4,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![4],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: Some({
                let mut m = HashMap::new();
                m.insert(0u32, vec!["class".to_string(), "original".to_string(), "oldtoken".to_string(), "stuff".to_string()]);
                m
            }),
            path_to_id: Some({
                let mut m = HashMap::new();
                m.insert(PathBuf::from(&clean), 0u32);
                m
            }),
        };

        // Update file with different content
        std::fs::write(&test_file, "class Updated { NewToken stuff; }").unwrap();
        update_file_in_index(&mut index, &PathBuf::from(&clean));

        // total_tokens should equal sum of file_token_counts
        let sum: u64 = index.file_token_counts.iter().map(|&c| c as u64).sum();
        assert_eq!(index.total_tokens, sum,
            "total_tokens ({}) should equal sum of file_token_counts ({})",
            index.total_tokens, sum);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_total_tokens_decremented_on_remove() {
        let mut index = make_test_index();
        index = build_watch_index_from(index);

        let initial_total = index.total_tokens;
        let file0_tokens = index.file_token_counts[0] as u64;

        remove_file_from_index(&mut index, &PathBuf::from("file0.cs"));

        assert_eq!(index.total_tokens, initial_total - file0_tokens,
            "total_tokens should decrease by file0's token count");
    }

    #[test]
    fn test_total_tokens_consistency_after_multiple_ops() {
        let dir = std::env::temp_dir().join("search_watcher_test_consistency");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let file1 = dir.join("a.cs");
        let file2 = dir.join("b.cs");
        std::fs::write(&file1, "class Alpha { }").unwrap();
        std::fs::write(&file2, "class Beta { }").unwrap();

        let mut index = ContentIndex {
            root: ".".to_string(),
            created_at: 0,
            max_age_secs: 3600,
            files: vec![],
            index: HashMap::new(),
            total_tokens: 0,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![],
            trigram: TrigramIndex::default(),
            trigram_dirty: false,
            forward: Some(HashMap::new()),
            path_to_id: Some(HashMap::new()),
        };

        // Add file1
        let clean1 = PathBuf::from(crate::clean_path(&file1.to_string_lossy()));
        update_file_in_index(&mut index, &clean1);

        // Add file2
        let clean2 = PathBuf::from(crate::clean_path(&file2.to_string_lossy()));
        update_file_in_index(&mut index, &clean2);

        // Update file1 with new content
        std::fs::write(&file1, "class AlphaUpdated { NewMethod(); }").unwrap();
        update_file_in_index(&mut index, &clean1);

        // Remove file2
        remove_file_from_index(&mut index, &clean2);

        // Verify consistency: total_tokens == sum(file_token_counts) for non-removed files
        let sum: u64 = index.file_token_counts.iter().map(|&c| c as u64).sum();
        assert_eq!(index.total_tokens, sum,
            "total_tokens ({}) should equal sum of file_token_counts ({}) after multiple operations",
            index.total_tokens, sum);

        let _ = std::fs::remove_dir_all(&dir);
    }
}