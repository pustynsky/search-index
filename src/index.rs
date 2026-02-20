//! Index storage: save/load/build for FileIndex and ContentIndex.

use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;

use crate::error::SearchError;
use search::{clean_path, extract_semantic_prefix, generate_trigrams, read_file_lossy, stable_hash, tokenize, ContentIndex, FileEntry, FileIndex, Posting, TrigramIndex};

use crate::{ContentIndexArgs, IndexArgs};

// ─── LZ4 compression helpers ────────────────────────────────────────

/// Magic bytes identifying LZ4-compressed index files.
pub const LZ4_MAGIC: &[u8; 4] = b"LZ4S";

/// Save a serializable value to a file with LZ4 frame compression.
/// Writes magic bytes, then LZ4-compressed bincode data.
/// Logs compression ratio and timing to stderr.
pub fn save_compressed<T: serde::Serialize>(path: &std::path::Path, data: &T, label: &str) -> Result<(), SearchError> {
    let start = Instant::now();

    let file = std::fs::File::create(path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(LZ4_MAGIC)?;
    let mut encoder = lz4_flex::frame::FrameEncoder::new(writer);
    bincode::serialize_into(&mut encoder, data)?;
    let mut writer = encoder.finish().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    writer.flush()?;

    let compressed_size = std::fs::metadata(path)?.len();
    let elapsed = start.elapsed();

    eprintln!("[{}] Saved {:.1} MB (compressed) in {:.2}s to {}",
        label,
        compressed_size as f64 / 1_048_576.0,
        elapsed.as_secs_f64(),
        path.display());

    Ok(())
}

/// Load a deserializable value from a file, supporting both LZ4-compressed
/// and legacy uncompressed formats (backward compatibility).
/// Returns `Err(SearchError::IndexLoad)` with a descriptive message on failure.
pub fn load_compressed<T: serde::de::DeserializeOwned>(path: &std::path::Path, label: &str) -> Result<T, SearchError> {
    let path_str = path.display().to_string();
    let start = Instant::now();
    let compressed_size = std::fs::metadata(path)
        .map_err(|e| SearchError::IndexLoad {
            path: path_str.clone(),
            message: format!("file not found or inaccessible: {}", e),
        })?
        .len();

    let file = std::fs::File::open(path).map_err(|e| SearchError::IndexLoad {
        path: path_str.clone(),
        message: format!("cannot open file: {}", e),
    })?;
    let mut reader = BufReader::new(file);

    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic).map_err(|e| SearchError::IndexLoad {
        path: path_str.clone(),
        message: format!("read error (magic bytes): {}", e),
    })?;

    let result = if &magic == LZ4_MAGIC {
        // Compressed format
        let decoder = lz4_flex::frame::FrameDecoder::new(reader);
        bincode::deserialize_from(decoder).map_err(|e| SearchError::IndexLoad {
            path: path_str.clone(),
            message: format!("LZ4 deserialization failed: {}", e),
        })?
    } else {
        // Legacy uncompressed format
        reader.seek(SeekFrom::Start(0)).map_err(|e| SearchError::IndexLoad {
            path: path_str.clone(),
            message: format!("seek error: {}", e),
        })?;
        let data = {
            let mut buf = Vec::new();
            reader.read_to_end(&mut buf).map_err(|e| SearchError::IndexLoad {
                path: path_str.clone(),
                message: format!("read error: {}", e),
            })?;
            buf
        };
        bincode::deserialize(&data).map_err(|e| SearchError::IndexLoad {
            path: path_str.clone(),
            message: format!("deserialization failed: {}", e),
        })?
    };

    let elapsed = start.elapsed();
    eprintln!("[{}] Loaded {:.1} MB in {:.3}s",
        label,
        compressed_size as f64 / 1_048_576.0,
        elapsed.as_secs_f64());

    Ok(result)
}

// ─── Index storage ───────────────────────────────────────────────────

/// Default production index directory: `%LOCALAPPDATA%/search-index`.
/// Tests should NOT use this — pass a test-local directory instead.
pub fn index_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("search-index")
}

pub fn index_path_for(dir: &str, index_base: &std::path::Path) -> PathBuf {
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let hash = stable_hash(&[canonical.to_string_lossy().as_bytes()]);
    let prefix = extract_semantic_prefix(&canonical);
    index_base.join(format!("{}_{:08x}.file-list", prefix, hash as u32))
}

pub fn save_index(index: &FileIndex, index_base: &std::path::Path) -> Result<(), SearchError> {
    fs::create_dir_all(index_base)?;
    let path = index_path_for(&index.root, index_base);
    save_compressed(&path, index, "file-index")
}

pub fn load_index(dir: &str, index_base: &std::path::Path) -> Result<FileIndex, SearchError> {
    let path = index_path_for(dir, index_base);
    load_compressed(&path, "file-index")
}

pub fn content_index_path_for(dir: &str, exts: &str, index_base: &std::path::Path) -> PathBuf {
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let hash = stable_hash(&[canonical.to_string_lossy().as_bytes(), exts.as_bytes()]);
    let prefix = extract_semantic_prefix(&canonical);
    index_base.join(format!("{}_{:08x}.word-search", prefix, hash as u32))
}

pub fn save_content_index(index: &ContentIndex, index_base: &std::path::Path) -> Result<(), SearchError> {
    fs::create_dir_all(index_base)?;
    let exts_str = index.extensions.join(",");
    let path = content_index_path_for(&index.root, &exts_str, index_base);
    save_compressed(&path, index, "content-index")
}

pub fn load_content_index(dir: &str, exts: &str, index_base: &std::path::Path) -> Result<ContentIndex, SearchError> {
    let path = content_index_path_for(dir, exts, index_base);
    load_compressed(&path, "content-index")
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
        if path.extension().is_some_and(|e| e == "word-search") {
            match load_compressed::<ContentIndex>(&path, "content-index") {
                Ok(index) => {
                    if index.root == clean {
                        return Some(index);
                    }
                }
                Err(e) => {
                    eprintln!("[find_content_index] Skipping {}: {}", path.display(), e);
                }
            }
        }
    }
    None
}

/// Read the root field from an index file without deserializing the whole file.
/// Handles both LZ4-compressed and legacy uncompressed formats.
/// Bincode stores a String as: u64 (length) + bytes. Since `root` is the first field in
/// FileIndex, ContentIndex, and DefinitionIndex, we can read just the first few bytes.
fn read_root_from_index_file(path: &std::path::Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;

    let reader: Box<dyn Read> = if &magic == LZ4_MAGIC {
        Box::new(lz4_flex::frame::FrameDecoder::new(BufReader::new(file)))
    } else {
        file.seek(SeekFrom::Start(0)).ok()?;
        Box::new(BufReader::new(file))
    };

    // Read bincode-encoded string: 8-byte length prefix + UTF-8 bytes
    let mut len_buf = [0u8; 8];
    let mut reader = reader;
    reader.read_exact(&mut len_buf).ok()?;
    let len = u64::from_le_bytes(len_buf) as usize;
    if len > 4096 { return None; }
    let mut str_buf = vec![0u8; len];
    reader.read_exact(&mut str_buf).ok()?;
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
            if !matches!(ext, Some("file-list") | Some("word-search") | Some("code-structure")) {
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

/// Remove all index files (.idx, .cidx, .didx) whose root matches the given directory.
/// Comparison is case-insensitive on the canonicalized paths (Windows-safe).
/// Returns the number of files removed.
pub fn cleanup_indexes_for_dir(dir: &str, index_base: &std::path::Path) -> usize {
    if !index_base.exists() {
        return 0;
    }

    let target = std::fs::canonicalize(dir)
        .map(|p| clean_path(&p.to_string_lossy()))
        .unwrap_or_else(|_| clean_path(dir));

    let mut removed = 0;

    if let Ok(entries) = std::fs::read_dir(index_base) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());
            if !matches!(ext, Some("file-list") | Some("word-search") | Some("code-structure")) {
                continue;
            }

            if let Some(root) = read_root_from_index_file(&path) {
                let root_canonical = std::fs::canonicalize(&root)
                    .map(|p| clean_path(&p.to_string_lossy()))
                    .unwrap_or_else(|_| clean_path(&root));
                if root_canonical.eq_ignore_ascii_case(&target) {
                    if std::fs::remove_file(&path).is_ok() {
                        removed += 1;
                        eprintln!("  Removed index for dir '{}': {} ({})",
                            dir, path.display(), ext.unwrap_or("?"));
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
        .unwrap_or(Duration::ZERO)
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
                match read_file_lossy(entry.path()) {
                    Ok((content, _was_lossy)) => {
                        file_data.lock().unwrap_or_else(|e| e.into_inner()).push((path, content));
                    }
                    Err(_) => {}
                }
            }
            ignore::WalkState::Continue
        })
    });

    let file_data = file_data.into_inner().unwrap();
    let file_count = file_data.len();
    let min_len = args.min_token_len;

    // ─── Parallel tokenization ──────────────────────────────────
    let num_tok_threads = thread_count.max(1);
    let tok_chunk_size = file_count.div_ceil(num_tok_threads).max(1);

    let chunk_results: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = file_data
            .chunks(tok_chunk_size)
            .enumerate()
            .map(|(chunk_idx, chunk)| {
                let base_file_id = (chunk_idx * tok_chunk_size) as u32;
                s.spawn(move || {
                    let mut local_files: Vec<String> = Vec::with_capacity(chunk.len());
                    let mut local_counts: Vec<u32> = Vec::with_capacity(chunk.len());
                    let mut local_index: HashMap<String, Vec<Posting>> = HashMap::new();
                    let mut local_total: u64 = 0;

                    for (i, (path, content)) in chunk.iter().enumerate() {
                        let file_id = base_file_id + i as u32;
                        local_files.push(path.clone());
                        let mut file_tokens: HashMap<String, Vec<u32>> = HashMap::new();
                        let mut file_total: u32 = 0;

                        for (line_num, line) in content.lines().enumerate() {
                            for token in tokenize(line, min_len) {
                                local_total += 1;
                                file_total += 1;
                                file_tokens
                                    .entry(token)
                                    .or_default()
                                    .push((line_num + 1) as u32);
                            }
                        }

                        local_counts.push(file_total);

                        for (token, lines) in file_tokens {
                            local_index
                                .entry(token)
                                .or_default()
                                .push(Posting { file_id, lines });
                        }
                    }

                    (local_files, local_counts, local_index, local_total)
                })
            })
            .collect();

        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // Free raw file contents — no longer needed after tokenization.
    // This releases ~1.6 GB for large repos (80K files × ~20KB avg content).
    // Without this drop, the file data stays alive until function return,
    // causing peak memory to be ~1.6 GB higher during build vs. load-from-disk.
    drop(file_data);

    // ─── Merge per-thread results ───────────────────────────────
    let mut files: Vec<String> = Vec::with_capacity(file_count);
    let mut file_token_counts: Vec<u32> = Vec::with_capacity(file_count);
    let mut index: HashMap<String, Vec<Posting>> = HashMap::new();
    let mut total_tokens: u64 = 0;

    for (local_files, local_counts, local_index, local_total) in chunk_results {
        files.extend(local_files);
        file_token_counts.extend(local_counts);
        total_tokens += local_total;
        for (token, postings) in local_index {
            index.entry(token).or_default().extend(postings);
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
        .unwrap_or(Duration::ZERO)
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

    // ─── LZ4 compression tests ──────────────────────────────

    #[test]
    fn test_save_load_compressed_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.bin");

        let data = vec!["hello".to_string(), "world".to_string(), "compressed".to_string()];
        crate::index::save_compressed(&path, &data, "test").unwrap();
        let loaded: Result<Vec<String>, _> = crate::index::load_compressed(&path, "test");
        assert!(loaded.is_ok());
        assert_eq!(data, loaded.unwrap());

        // Verify file starts with LZ4 magic bytes
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(&raw[..4], crate::index::LZ4_MAGIC);
    }

    #[test]
    fn test_load_compressed_legacy_uncompressed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("legacy.bin");

        // Write uncompressed bincode (legacy format)
        let data = vec!["legacy".to_string(), "format".to_string()];
        let encoded = bincode::serialize(&data).unwrap();
        std::fs::write(&path, &encoded).unwrap();

        // load_compressed should still read it via backward compatibility
        let loaded: Result<Vec<String>, _> = crate::index::load_compressed(&path, "test");
        assert!(loaded.is_ok());
        assert_eq!(data, loaded.unwrap());
    }

    #[test]
    fn test_load_compressed_missing_file_returns_err() {
        let path = std::path::Path::new("/nonexistent/path/to/file.bin");
        let result: Result<Vec<String>, _> = crate::index::load_compressed(path, "test");
        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("Failed to load index"), "Error should contain 'Failed to load index', got: {}", err_msg);
    }

    #[test]
    fn test_load_compressed_corrupt_data() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("corrupt.bin");

        // Write random bytes that look like neither valid LZ4 nor valid bincode
        std::fs::write(&path, b"this is not valid data at all!!!!!").unwrap();
        let result: Result<Vec<String>, _> = crate::index::load_compressed(&path, "test");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("deserialization failed"), "Error should mention deserialization, got: {}", err_msg);
    }

    #[test]
    fn test_compressed_file_smaller_than_uncompressed() {
        let tmp = tempfile::tempdir().unwrap();
        let compressed_path = tmp.path().join("compressed.bin");
        let uncompressed_path = tmp.path().join("uncompressed.bin");

        // Create data with repetitive content (compresses well)
        let data: Vec<String> = (0..1000).map(|i| format!("repeated_token_{}", i % 10)).collect();

        crate::index::save_compressed(&compressed_path, &data, "test").unwrap();
        let uncompressed = bincode::serialize(&data).unwrap();
        std::fs::write(&uncompressed_path, &uncompressed).unwrap();

        let compressed_size = std::fs::metadata(&compressed_path).unwrap().len();
        let uncompressed_size = std::fs::metadata(&uncompressed_path).unwrap().len();

        assert!(compressed_size < uncompressed_size,
            "Compressed ({}) should be smaller than uncompressed ({})",
            compressed_size, uncompressed_size);
    }
}