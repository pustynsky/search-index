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
use search::{clean_path, tokenize, ContentIndex, FileEntry, FileIndex, Posting};

use crate::{ContentIndexArgs, IndexArgs};

// ─── Index storage ───────────────────────────────────────────────────

pub fn index_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("search-index")
}

pub fn index_path_for(dir: &str) -> PathBuf {
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let mut hasher = DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut hasher);
    let hash = hasher.finish();
    index_dir().join(format!("{:016x}.idx", hash))
}

pub fn save_index(index: &FileIndex) -> Result<(), SearchError> {
    let dir = index_dir();
    fs::create_dir_all(&dir)?;
    let path = index_path_for(&index.root);
    let encoded = bincode::serialize(index)?;
    fs::write(&path, encoded)?;
    Ok(())
}

pub fn load_index(dir: &str) -> Option<FileIndex> {
    let path = index_path_for(dir);
    let data = fs::read(&path).ok()?;
    bincode::deserialize(&data).ok()
}

pub fn content_index_path_for(dir: &str, exts: &str) -> PathBuf {
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let mut hasher = DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut hasher);
    exts.hash(&mut hasher);
    let hash = hasher.finish();
    index_dir().join(format!("{:016x}.cidx", hash))
}

pub fn save_content_index(index: &ContentIndex) -> Result<(), SearchError> {
    let dir = index_dir();
    fs::create_dir_all(&dir)?;
    let exts_str = index.extensions.join(",");
    let path = content_index_path_for(&index.root, &exts_str);
    let encoded = bincode::serialize(index)?;
    fs::write(&path, encoded)?;
    Ok(())
}

pub fn load_content_index(dir: &str, exts: &str) -> Option<ContentIndex> {
    let path = content_index_path_for(dir, exts);
    let data = fs::read(&path).ok()?;
    bincode::deserialize(&data).ok()
}

/// Try to find any content index (.cidx) file matching the given directory
pub fn find_content_index_for_dir(dir: &str) -> Option<ContentIndex> {
    let idx_dir = index_dir();
    if !idx_dir.exists() {
        return None;
    }
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let clean = clean_path(&canonical.to_string_lossy());

    for entry in fs::read_dir(&idx_dir).ok()?.flatten() {
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
        forward: None,
        path_to_id: None,
    }
}