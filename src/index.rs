//! Index storage: save/load/build for FileIndex and ContentIndex.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;

use crate::error::SearchError;
use search::{clean_path, generate_trigrams, tokenize, ContentIndex, FileEntry, FileIndex, Posting, TrigramIndex};

use crate::{ContentIndexArgs, IndexArgs};

// ─── Index storage ───────────────────────────────────────────────────

/// Default production index directory: `%LOCALAPPDATA%/search-index`.
/// Tests should NOT use this — pass a test-local directory instead.
pub fn index_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("search-index")
}

pub fn index_path_for(dir: &str, index_base: &std::path::Path) -> PathBuf {
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let mut hasher = DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut hasher);
    let hash = hasher.finish();
    index_base.join(format!("{:016x}.idx", hash))
}

pub fn save_index(index: &FileIndex, index_base: &std::path::Path) -> Result<(), SearchError> {
    fs::create_dir_all(index_base)?;
    let path = index_path_for(&index.root, index_base);
    let encoded = bincode::serialize(index)?;
    fs::write(&path, encoded)?;
    Ok(())
}

pub fn load_index(dir: &str, index_base: &std::path::Path) -> Option<FileIndex> {
    let path = index_path_for(dir, index_base);
    let data = fs::read(&path).ok()?;
    bincode::deserialize(&data).ok()
}

pub fn content_index_path_for(dir: &str, exts: &str, index_base: &std::path::Path) -> PathBuf {
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let mut hasher = DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut hasher);
    exts.hash(&mut hasher);
    let hash = hasher.finish();
    index_base.join(format!("{:016x}.cidx", hash))
}

pub fn save_content_index(index: &ContentIndex, index_base: &std::path::Path) -> Result<(), SearchError> {
    fs::create_dir_all(index_base)?;
    let exts_str = index.extensions.join(",");
    let path = content_index_path_for(&index.root, &exts_str, index_base);
    let encoded = bincode::serialize(index)?;
    fs::write(&path, encoded)?;
    Ok(())
}

pub fn load_content_index(dir: &str, exts: &str, index_base: &std::path::Path) -> Option<ContentIndex> {
    let path = content_index_path_for(dir, exts, index_base);
    let data = fs::read(&path).ok()?;
    bincode::deserialize(&data).ok()
}

/// Try to find any content index (.cidx) file matching the given directory
pub fn find_content_index_for_dir(dir: &str, index_base: &std::path::Path) -> Option<ContentIndex> {
    if !index_base.exists() {
        return None;
    }
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let clean = clean_path(&canonical.to_string_lossy());

    for entry in fs::read_dir(index_base).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("cidx")
            && let Ok(data) = fs::read(&path)
                && let Ok(index) = bincode::deserialize::<ContentIndex>(&data)
                    && index.root == clean {
                        return Some(index);
                    }
    }
    None
}

/// Read the root field from a bincode-serialized index file without deserializing the whole file.
/// Bincode stores a String as: u64 (length) + bytes. Since `root` is the first field in
/// FileIndex, ContentIndex, and DefinitionIndex, we can read just the first few bytes.
fn read_root_from_index_file(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut len_buf = [0u8; 8];
    file.read_exact(&mut len_buf).ok()?;
    let str_len = u64::from_le_bytes(len_buf) as usize;
    // Sanity check: root paths shouldn't be longer than 4KB
    if str_len > 4096 {
        return None;
    }
    let mut str_buf = vec![0u8; str_len];
    file.read_exact(&mut str_buf).ok()?;
    String::from_utf8(str_buf).ok()
}

/// Remove orphaned index files whose root directory no longer exists on disk.
/// Returns the number of files removed.
/// Reads only the root field from each file header (fast — no full deserialization).
pub fn cleanup_orphaned_indexes(index_base: &std::path::Path) -> usize {
    if !index_base.exists() {
        return 0;
    }

    let mut removed = 0;

    if let Ok(entries) = std::fs::read_dir(index_base) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());
            if !matches!(ext, Some("idx") | Some("cidx") | Some("didx")) {
                continue;
            }

            if let Some(root) = read_root_from_index_file(&path) {
                if !std::path::Path::new(&root).exists() {
                    if std::fs::remove_file(&path).is_ok() {
                        removed += 1;
                        eprintln!("  Removed orphaned index: {} (root: {})", path.display(), root);
                    }
                }
            }
        }
    }

    removed
}

// ─── Index building ──────────────────────────────────────────────────

pub fn build_index(args: &IndexArgs) -> FileIndex {
    let root = fs::canonicalize(&args.dir).unwrap_or_else(|_| PathBuf::from(&args.dir));
    let root_str = clean_path(&root.to_string_lossy());

    eprintln!("Indexing {}...", root_str);
    let start = Instant::now();

    let mut builder = WalkBuilder::new(&root);
    builder.hidden(!args.hidden);
    builder.git_ignore(!args.no_ignore);
    builder.git_global(!args.no_ignore);
    builder.git_exclude(!args.no_ignore);

    let thread_count = if args.threads == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    } else {
        args.threads
    };
    builder.threads(thread_count);

    let entries: Mutex<Vec<FileEntry>> = Mutex::new(Vec::new());

    builder.build_parallel().run(|| {
        let entries = &entries;
        Box::new(move |result| {
            if let Ok(entry) = result {
                let path = clean_path(&entry.path().to_string_lossy());
                let metadata = entry.metadata().ok();
                let (size, modified, is_dir) = if let Some(m) = metadata {
                    let mod_time = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    (m.len(), mod_time, m.is_dir())
                } else {
                    (0, 0, false)
                };

                let fe = FileEntry {
                    path,
                    size,
                    modified,
                    is_dir,
                };

                entries.lock().unwrap_or_else(|e| e.into_inner()).push(fe);
            }
            ignore::WalkState::Continue
        })
    });

    let entries = entries.into_inner().unwrap();
    let count = entries.len();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let index = FileIndex {
        root: root_str,
        created_at: now,
        max_age_secs: args.max_age_hours * 3600,
        entries,
    };

    let elapsed = start.elapsed();
    eprintln!(
        "Indexed {} entries in {:.3}s",
        count,
        elapsed.as_secs_f64()
    );

    index
}

// ─── Content index building ──────────────────────────────────────────

pub fn build_content_index(args: &ContentIndexArgs) -> ContentIndex {
    let root = fs::canonicalize(&args.dir).unwrap_or_else(|_| PathBuf::from(&args.dir));
    let root_str = clean_path(&root.to_string_lossy());
    let extensions: Vec<String> = args.ext.split(',').map(|s| s.trim().to_lowercase()).collect();

    eprintln!(
        "Building content index for {} (extensions: {})...",
        root_str,
        extensions.join(", ")
    );
    let start = Instant::now();

    let mut builder = WalkBuilder::new(&root);
    builder.hidden(!args.hidden);
    builder.git_ignore(!args.no_ignore);
    builder.git_global(!args.no_ignore);
    builder.git_exclude(!args.no_ignore);

    let thread_count = if args.threads == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    } else {
        args.threads
    };
    builder.threads(thread_count);

    let file_data: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

    builder.build_parallel().run(|| {
        let extensions = extensions.clone();
        let file_data = &file_data;
        Box::new(move |result| {
            if let Ok(entry) = result {
                if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                    return ignore::WalkState::Continue;
                }
                let ext_match = entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)));
                if !ext_match {
                    return ignore::WalkState::Continue;
                }
                let path = clean_path(&entry.path().to_string_lossy());
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    file_data.lock().unwrap_or_else(|e| e.into_inner()).push((path, content));
                }
            }
            ignore::WalkState::Continue
        })
    });

    let file_data = file_data.into_inner().unwrap();
    let file_count = file_data.len();
    let min_len = args.min_token_len;

    let mut files: Vec<String> = Vec::with_capacity(file_count);
    let mut file_token_counts: Vec<u32> = Vec::with_capacity(file_count);
    let mut index: HashMap<String, Vec<Posting>> = HashMap::new();
    let mut total_tokens: u64 = 0;

    for (path, content) in &file_data {
        let file_id = files.len() as u32;
        files.push(path.clone());
        let mut file_tokens: HashMap<String, Vec<u32>> = HashMap::new();
        let mut file_total: u32 = 0;

        for (line_num, line) in content.lines().enumerate() {
            for token in tokenize(line, min_len) {
                total_tokens += 1;
                file_total += 1;
                file_tokens.entry(token).or_default().push((line_num + 1) as u32);
            }
        }

        file_token_counts.push(file_total);

        for (token, lines) in file_tokens {
            index.entry(token).or_default().push(Posting { file_id, lines });
        }
    }

    let unique_tokens = index.len();

    // Build trigram index from inverted index tokens
    let trigram = build_trigram_index(&index);
    eprintln!(
        "Trigram index: {} trigrams, {} tokens",
        trigram.trigram_map.len(),
        trigram.tokens.len()
    );

    let elapsed = start.elapsed();

    eprintln!(
        "Indexed {} files, {} unique tokens ({} total) in {:.3}s",
        file_count, unique_tokens, total_tokens, elapsed.as_secs_f64()
    );

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    ContentIndex {
        root: root_str,
        created_at: now,
        max_age_secs: args.max_age_hours * 3600,
        files,
        index,
        total_tokens,
        extensions,
        file_token_counts,
        trigram,
        trigram_dirty: false,
        forward: None,
        path_to_id: None,
    }
}

/// Build a trigram index from the inverted index's token keys.
pub fn build_trigram_index(inverted: &HashMap<String, Vec<Posting>>) -> TrigramIndex {
    let mut tokens: Vec<String> = inverted.keys().cloned().collect();
    tokens.sort();

    let mut trigram_map: HashMap<String, Vec<u32>> = HashMap::new();

    for (idx, token) in tokens.iter().enumerate() {
        let trigrams = generate_trigrams(token);
        for trigram in trigrams {
            trigram_map.entry(trigram).or_default().push(idx as u32);
        }
    }

    // Sort and dedup posting lists
    for list in trigram_map.values_mut() {
        list.sort();
        list.dedup();
    }

    TrigramIndex { tokens, trigram_map }
}
#[cfg(test)]
mod index_tests {
    use std::collections::HashMap;
    use search::Posting;
    use crate::index::build_trigram_index;

    #[test]
    fn test_build_trigram_index_basic() {
        let mut inverted: HashMap<String, Vec<Posting>> = HashMap::new();
        inverted.insert("httpclient".to_string(), vec![Posting { file_id: 0, lines: vec![1] }]);
        inverted.insert("httphandler".to_string(), vec![Posting { file_id: 1, lines: vec![5] }]);
        inverted.insert("ab".to_string(), vec![Posting { file_id: 2, lines: vec![10] }]); // too short for trigrams

        let ti = build_trigram_index(&inverted);

        // Tokens should be sorted
        assert_eq!(ti.tokens, vec!["ab", "httpclient", "httphandler"]);

        // "htt" should map to both http tokens
        let htt = ti.trigram_map.get("htt").unwrap();
        assert_eq!(htt.len(), 2); // indices of httpclient and httphandler

        // "cli" should only map to httpclient
        let cli = ti.trigram_map.get("cli").unwrap();
        assert_eq!(cli.len(), 1);

        // "ab" should not generate any trigrams (too short)
        // but "ab" should still be in tokens list
        assert!(ti.tokens.contains(&"ab".to_string()));
    }

    #[test]
    fn test_build_trigram_index_empty() {
        let inverted: HashMap<String, Vec<Posting>> = HashMap::new();
        let ti = build_trigram_index(&inverted);
        assert!(ti.tokens.is_empty());
        assert!(ti.trigram_map.is_empty());
    }

    #[test]
    fn test_build_trigram_index_sorted_posting_lists() {
        let mut inverted: HashMap<String, Vec<Posting>> = HashMap::new();
        inverted.insert("abcdef".to_string(), vec![Posting { file_id: 0, lines: vec![1] }]);
        inverted.insert("abcxyz".to_string(), vec![Posting { file_id: 1, lines: vec![2] }]);

        let ti = build_trigram_index(&inverted);

        // All posting lists should be sorted
        for (_, list) in &ti.trigram_map {
            for window in list.windows(2) {
                assert!(window[0] <= window[1], "Posting list not sorted");
            }
        }
    }

    #[test]
    fn test_build_trigram_index_single_token() {
        let mut inverted: HashMap<String, Vec<Posting>> = HashMap::new();
        inverted.insert("foobar".to_string(), vec![Posting { file_id: 0, lines: vec![1] }]);

        let ti = build_trigram_index(&inverted);

        assert_eq!(ti.tokens, vec!["foobar"]);
        // "foobar" has 4 trigrams: foo, oob, oba, bar
        assert_eq!(ti.trigram_map.len(), 4);
        assert!(ti.trigram_map.contains_key("foo"));
        assert!(ti.trigram_map.contains_key("oob"));
        assert!(ti.trigram_map.contains_key("oba"));
        assert!(ti.trigram_map.contains_key("bar"));
    }

    #[test]
    fn test_build_trigram_index_deduplicates() {
        // Two tokens sharing the same trigram should appear once each in the posting list
        let mut inverted: HashMap<String, Vec<Posting>> = HashMap::new();
        inverted.insert("abc".to_string(), vec![Posting { file_id: 0, lines: vec![1] }]);
        inverted.insert("abcdef".to_string(), vec![Posting { file_id: 1, lines: vec![2] }]);

        let ti = build_trigram_index(&inverted);

        let abc_list = ti.trigram_map.get("abc").unwrap();
        // Both "abc" (idx 0) and "abcdef" (idx 1) share trigram "abc"
        assert_eq!(abc_list.len(), 2);
        // Should be deduped (no duplicates)
        let mut deduped = abc_list.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(abc_list.len(), deduped.len());
    }
}