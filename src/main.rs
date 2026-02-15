use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use ignore::WalkBuilder;
use regex::Regex;
use tracing::{info, warn};

// Re-export core types from library crate
pub use search::{clean_path, tokenize, ContentIndex, FileEntry, FileIndex, Posting};

mod definitions;
mod mcp;

// ─── CLI ─────────────────────────────────────────────────────────────

/// Fast file search tool with optional indexing for instant lookups
#[derive(Parser, Debug)]
#[command(name = "search", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Search for files (live filesystem walk)
    Find(FindArgs),

    /// Build a file index for a directory
    Index(IndexArgs),

    /// Search using a pre-built index (instant results)
    Fast(FastArgs),

    /// Show index info or list indexed directories
    Info,

    /// Build an inverted (content) index for text/code files
    ContentIndex(ContentIndexArgs),

    /// Search file contents using inverted index (instant grep).
    /// Requires a content index built with 'content-index' command.
    /// Results are ranked by TF-IDF relevance score.
    /// Supports: single term, multi-term (comma-separated), regex (--regex).
    ///
    /// Examples:
    ///   search grep "HttpClient" -d C:\Projects -e cs
    ///   search grep "HttpClient,ILogger" -d C:\Projects -e cs --all
    ///   search grep "i.*cache" -d C:\Projects -e cs --regex
    Grep(GrepArgs),

    /// Start MCP (Model Context Protocol) server over stdio.
    /// Loads content index into memory for instant queries.
    /// AI agents connect via JSON-RPC 2.0 over stdin/stdout.
    Serve(ServeArgs),

    /// Build a code definition index (classes, methods, interfaces, stored procedures, etc.)
    /// Uses tree-sitter to parse C# and SQL files and extract structural definitions.
    /// Index is saved to disk for fast loading.
    DefIndex(definitions::DefIndexArgs),
}

#[derive(Parser, Debug)]
struct FindArgs {
    /// Search pattern (substring or regex with --regex)
    pattern: String,

    /// Root directory to search in
    #[arg(short, long, default_value = ".")]
    dir: String,

    /// Treat pattern as a regular expression
    #[arg(short, long)]
    regex: bool,

    /// Search file contents instead of file names
    #[arg(long)]
    contents: bool,

    /// Show hidden files
    #[arg(long)]
    hidden: bool,

    /// Maximum search depth (0 = unlimited)
    #[arg(long, default_value = "0")]
    max_depth: usize,

    /// Number of parallel threads (0 = auto)
    #[arg(short, long, default_value = "0")]
    threads: usize,

    /// Case-insensitive search
    #[arg(short = 'i', long)]
    ignore_case: bool,

    /// Also search .gitignore'd files
    #[arg(long)]
    no_ignore: bool,

    /// Show only the count of matches
    #[arg(short = 'c', long)]
    count: bool,

    /// File extension filter
    #[arg(short, long)]
    ext: Option<String>,
}

#[derive(Parser, Debug)]
pub struct IndexArgs {
    /// Directory to index
    #[arg(short, long, default_value = ".")]
    pub dir: String,

    /// Max index age in hours before auto-reindex (default: 24)
    #[arg(long, default_value = "24")]
    pub max_age_hours: u64,

    /// Include hidden files in index
    #[arg(long)]
    pub hidden: bool,

    /// Include .gitignore'd files
    #[arg(long)]
    pub no_ignore: bool,

    /// Number of parallel threads (0 = auto)
    #[arg(short, long, default_value = "0")]
    pub threads: usize,
}

#[derive(Parser, Debug)]
struct FastArgs {
    /// Search pattern (substring or regex with --regex)
    pattern: String,

    /// Directory whose index to search (must be indexed first)
    #[arg(short, long, default_value = ".")]
    dir: String,

    /// Treat pattern as a regular expression
    #[arg(short, long)]
    regex: bool,

    /// Case-insensitive search
    #[arg(short = 'i', long)]
    ignore_case: bool,

    /// Show only the count of matches
    #[arg(short = 'c', long)]
    count: bool,

    /// File extension filter
    #[arg(short, long)]
    ext: Option<String>,

    /// Auto-reindex if index is stale
    #[arg(long, default_value = "true")]
    auto_reindex: bool,

    /// Search only directories
    #[arg(long)]
    dirs_only: bool,

    /// Search only files
    #[arg(long)]
    files_only: bool,

    /// Minimum file size in bytes
    #[arg(long)]
    min_size: Option<u64>,

    /// Maximum file size in bytes
    #[arg(long)]
    max_size: Option<u64>,
}

#[derive(Parser, Debug)]
#[command(after_long_help = r#"WHAT IS MCP:
  Model Context Protocol (MCP) is a JSON-RPC 2.0 protocol over stdio that
  allows AI agents (VS Code Copilot, Roo/Cline, Claude) to call tools natively.
  The server reads JSON requests from stdin and writes responses to stdout.

EXAMPLES:
  Basic:          search serve --dir C:\Projects\MyApp --ext cs
  Multi-ext:      search serve --dir C:\Projects --ext cs,sql,csproj
  With watcher:   search serve --dir C:\Projects --ext cs --watch
  With defs:      search serve --dir C:\Projects --ext cs --watch --definitions
  Custom debounce: search serve --dir . --ext rs --watch --debounce-ms 1000

VS CODE CONFIGURATION (.vscode/mcp.json):
  {
    "servers": {
      "search-index": {
        "command": "search",
        "args": ["serve", "--dir", "C:\\Projects\\MyApp", "--ext", "cs", "--watch", "--definitions"]
      }
    }
  }

AVAILABLE TOOLS (exposed via MCP):
  search_grep        -- Search content index (TF-IDF ranked, regex, phrase, multi-term)
  search_definitions -- Search code definitions: classes, methods, interfaces, enums, SPs
                       Supports containsLine to find which method/class contains a line.
                       (requires --definitions flag)
  search_callers     -- Find all callers of a method and build a call tree (up/down).
                       Combines grep index + AST index. Replaces 7+ manual queries with 1.
                       (requires --definitions flag)
  search_find        -- Live filesystem search (no index, slow for large dirs)
  search_fast        -- Search file name index (instant)
  search_info        -- Show all indexes
  search_reindex     -- Force rebuild + reload index

HOW IT WORKS:
  1. On startup: loads (or builds) content index into RAM (~0.8s one-time)
  2. With --definitions: loads cached definition index from disk (instant),
     or builds it using tree-sitter on first use (~14s for 48K files)
  3. Starts JSON-RPC event loop on stdin/stdout
  4. All search queries use in-memory index (~0.001s per query)
  5. With --watch: file changes update both indexes incrementally (~5ms/file)
  6. Logging goes to stderr (never pollutes JSON-RPC on stdout)
"#)]
pub struct ServeArgs {
    /// Directory to index and serve. Index is loaded into RAM at startup.
    /// All search_grep queries will search this directory's content index.
    #[arg(short, long, default_value = ".")]
    pub dir: String,

    /// File extensions to index, comma-separated (e.g. "cs,sql,csproj").
    /// One combined index is built for all extensions.
    /// Individual tool calls can filter results by extension.
    #[arg(short, long, default_value = "cs")]
    pub ext: String,

    /// Watch for file changes and update index incrementally (~5ms per file).
    /// Without this flag, the index is static (loaded once at startup).
    /// Uses OS file notifications (ReadDirectoryChangesW on Windows).
    #[arg(long)]
    pub watch: bool,

    /// Debounce delay in ms for file watcher. Multiple rapid saves are batched.
    /// Lower = more responsive but more CPU. Higher = fewer updates.
    #[arg(long, default_value = "500")]
    pub debounce_ms: u64,

    /// Log level for stderr output (error, warn, info, debug)
    #[arg(long, default_value = "info")]
    pub log_level: String,

    /// If more than N files change in one debounce window, do a full reindex
    /// instead of incremental updates. Handles git checkout/pull scenarios.
    #[arg(long, default_value = "100")]
    pub bulk_threshold: usize,

    /// Also load (or build) a code definition index (classes, interfaces,
    /// methods, properties, fields, enums, stored procedures, tables, etc.)
    /// using tree-sitter AST parsing of C# and SQL files.
    /// Enables the search_definitions and search_callers MCP tools.
    /// On startup: loads cached index from disk if available (instant).
    /// Only builds from scratch on first use (~14s for 48K files).
    /// With --watch, file changes update the def index incrementally.
    /// Use 'search def-index' CLI or search_reindex tool to force rebuild.
    #[arg(long)]
    pub definitions: bool,
}

#[derive(Parser, Debug)]
pub struct ContentIndexArgs {
    /// Directory to index
    #[arg(short, long, default_value = ".")]
    pub dir: String,

    /// File extensions to index (comma-separated, e.g. "cs,rs,py,js")
    #[arg(short, long, default_value = "cs")]
    pub ext: String,

    /// Max index age in hours before auto-reindex (default: 24)
    #[arg(long, default_value = "24")]
    pub max_age_hours: u64,

    /// Include hidden files
    #[arg(long)]
    pub hidden: bool,

    /// Include .gitignore'd files
    #[arg(long)]
    pub no_ignore: bool,

    /// Number of parallel threads (0 = auto)
    #[arg(short, long, default_value = "0")]
    pub threads: usize,

    /// Minimum token length to index (default: 2)
    #[arg(long, default_value = "2")]
    pub min_token_len: usize,
}

#[derive(Parser, Debug)]
#[command(after_long_help = r#"EXAMPLES:
  Single term:     search grep "HttpClient" -d C:\Projects -e cs
  Multi-term OR:   search grep "HttpClient,ILogger,Task" -d C:\Projects -e cs
  Multi-term AND:  search grep "HttpClient,ILogger" -d C:\Projects -e cs --all
  Regex:           search grep "i.*cache" -d C:\Projects -e cs --regex
  Regex + lines:   search grep ".*factory" -d C:\Projects -e cs --regex --show-lines
  Top 10 results:  search grep "HttpClient" -d C:\Projects --max-results 10
  Exclude dirs:    search grep "HttpClient" -d . -e cs --exclude-dir test --exclude-dir E2E
  Exclude files:   search grep "HttpClient" -d . -e cs --exclude Mock
  Context lines:   search grep "HttpClient" -d . -e cs --show-lines -C 3
  Before/after:    search grep "HttpClient" -d . -e cs --show-lines -B 2 -A 5

NOTES:
  - Requires a content index. Build one first:
      search content-index -d C:\Projects -e cs,rs,py
  - Results sorted by TF-IDF relevance (most relevant files first)
  - Multi-term: comma-separated, OR by default, AND with --all
  - Regex: pattern matched against all indexed tokens (e.g. 754K unique tokens)
  - Use --show-lines to see actual source code lines from matching files
  - --exclude-dir and --exclude filter results by path substring (case-insensitive)
  - Context lines (-C/-B/-A) show surrounding code, like grep -C
"#)]
struct GrepArgs {
    /// Search term(s). Comma-separated for multi-term: "HttpClient,ILogger,Task".
    /// With --regex: pattern matched against all indexed tokens, e.g. "i.*cache"
    pattern: String,

    /// Directory whose content index to search (must be indexed first with content-index)
    #[arg(short, long, default_value = ".")]
    dir: String,

    /// Show only the count of matching files (no file list)
    #[arg(short = 'c', long)]
    count: bool,

    /// Read matching files from disk and show the actual source code lines
    #[arg(long)]
    show_lines: bool,

    /// Automatically rebuild content index if it's older than max-age-hours
    #[arg(long, default_value = "true")]
    auto_reindex: bool,

    /// Filter results by file extension (e.g. "cs", "sql", "csproj")
    #[arg(short, long)]
    ext: Option<String>,

    /// Maximum number of results to display (0 = show all)
    #[arg(long, default_value = "0")]
    max_results: usize,

    /// AND mode: file must contain ALL comma-separated terms.
    /// Default is OR mode (file matches if it contains ANY term).
    /// Example: search grep "HttpClient,ILogger" --all → files with BOTH terms
    #[arg(long)]
    all: bool,

    /// Treat pattern as regex. Matches against all indexed tokens (Rust regex syntax).
    /// Example: "i.*cache" matches itenantcache, iusercache, etc.
    /// Example: ".*async$" matches getasync, postasync, sendasync
    #[arg(short, long)]
    regex: bool,

    /// Exclude files whose path contains this substring (can be used multiple times).
    /// Example: --exclude-dir test --exclude-dir E2E → skip test and E2E directories
    #[arg(long, action = clap::ArgAction::Append)]
    exclude_dir: Vec<String>,

    /// Exclude files matching this pattern in their path (substring match).
    /// Example: --exclude Mock → skip files with "Mock" in name
    #[arg(long, action = clap::ArgAction::Append)]
    exclude: Vec<String>,

    /// Show N lines of context around each match (like grep -C).
    /// Only works with --show-lines
    #[arg(short = 'C', long, default_value = "0")]
    context: usize,

    /// Show N lines before each match (like grep -B).
    /// Only works with --show-lines
    #[arg(short = 'B', long, default_value = "0")]
    before: usize,

    /// Show N lines after each match (like grep -A).
    /// Only works with --show-lines
    #[arg(short = 'A', long, default_value = "0")]
    after: usize,

    /// Phrase search: find files containing the exact phrase.
    /// Tokenizes phrase, uses index to find candidate files (AND), then verifies
    /// by reading files and checking for the exact substring.
    /// Example: search grep "new HttpClient(" -d . -e cs --phrase
    #[arg(long)]
    phrase: bool,
}

// ─── Index storage ───────────────────────────────────────────────────

fn index_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("search-index")
}

fn index_path_for(dir: &str) -> PathBuf {
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let mut hasher = DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut hasher);
    let hash = hasher.finish();
    index_dir().join(format!("{:016x}.idx", hash))
}

pub fn save_index(index: &FileIndex) -> Result<(), Box<dyn std::error::Error>> {
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

fn content_index_path_for(dir: &str, exts: &str) -> PathBuf {
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let mut hasher = DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut hasher);
    exts.hash(&mut hasher);
    let hash = hasher.finish();
    index_dir().join(format!("{:016x}.cidx", hash))
}

pub fn save_content_index(index: &ContentIndex) -> Result<(), Box<dyn std::error::Error>> {
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
        if path.extension().and_then(|e| e.to_str()) == Some("cidx") {
            if let Ok(data) = fs::read(&path) {
                if let Ok(index) = bincode::deserialize::<ContentIndex>(&data) {
                    if index.root == clean {
                        return Some(index);
                    }
                }
            }
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

                entries.lock().unwrap().push(fe);
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

// ─── Live search (no index) ──────────────────────────────────────────

fn cmd_find(args: FindArgs) {
    let start = Instant::now();

    let pattern = if args.ignore_case {
        args.pattern.to_lowercase()
    } else {
        args.pattern.clone()
    };

    let re = if args.regex {
        match Regex::new(&if args.ignore_case {
            format!("(?i){}", &args.pattern)
        } else {
            args.pattern.clone()
        }) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("Invalid regex: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    let root = Path::new(&args.dir);
    if !root.exists() {
        eprintln!("Directory does not exist: {}", args.dir);
        std::process::exit(1);
    }

    let match_count = AtomicUsize::new(0);
    let file_count = AtomicUsize::new(0);

    let mut builder = WalkBuilder::new(root);
    builder.hidden(!args.hidden);
    builder.git_ignore(!args.no_ignore);
    builder.git_global(!args.no_ignore);
    builder.git_exclude(!args.no_ignore);

    if args.max_depth > 0 {
        builder.max_depth(Some(args.max_depth));
    }

    let thread_count = if args.threads == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    } else {
        args.threads
    };
    builder.threads(thread_count);

    if args.contents {
        builder.build_parallel().run(|| {
            let pattern = pattern.clone();
            let re = re.clone();
            let ignore_case = args.ignore_case;
            let count_only = args.count;
            let ext_filter = args.ext.clone();
            let match_count = &match_count;
            let file_count = &file_count;

            Box::new(move |entry| {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => return ignore::WalkState::Continue,
                };

                if !entry.file_type().map_or(false, |ft| ft.is_file()) {
                    return ignore::WalkState::Continue;
                }

                if let Some(ref ext) = ext_filter {
                    let matches_ext = entry
                        .path()
                        .extension()
                        .and_then(|e| e.to_str())
                        .map_or(false, |e| e.eq_ignore_ascii_case(ext));
                    if !matches_ext {
                        return ignore::WalkState::Continue;
                    }
                }

                file_count.fetch_add(1, Ordering::Relaxed);

                let content = match fs::read_to_string(entry.path()) {
                    Ok(c) => c,
                    Err(_) => return ignore::WalkState::Continue,
                };

                let matched = if let Some(ref re) = re {
                    re.is_match(&content)
                } else if ignore_case {
                    content.to_lowercase().contains(&pattern)
                } else {
                    content.contains(&pattern)
                };

                if matched {
                    match_count.fetch_add(1, Ordering::Relaxed);
                    if !count_only {
                        for (line_num, line) in content.lines().enumerate() {
                            let line_matched = if let Some(ref re) = re {
                                re.is_match(line)
                            } else if ignore_case {
                                line.to_lowercase().contains(&pattern)
                            } else {
                                line.contains(&pattern)
                            };
                            if line_matched {
                                println!(
                                    "{}:{}: {}",
                                    entry.path().display(),
                                    line_num + 1,
                                    line.trim()
                                );
                            }
                        }
                    }
                }

                ignore::WalkState::Continue
            })
        });
    } else {
        builder.build_parallel().run(|| {
            let pattern = pattern.clone();
            let re = re.clone();
            let ignore_case = args.ignore_case;
            let count_only = args.count;
            let ext_filter = args.ext.clone();
            let match_count = &match_count;
            let file_count = &file_count;

            Box::new(move |entry| {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => return ignore::WalkState::Continue,
                };

                file_count.fetch_add(1, Ordering::Relaxed);

                let name = match entry.file_name().to_str() {
                    Some(n) => n.to_string(),
                    None => return ignore::WalkState::Continue,
                };

                if let Some(ref ext) = ext_filter {
                    let matches_ext = entry
                        .path()
                        .extension()
                        .and_then(|e| e.to_str())
                        .map_or(false, |e| e.eq_ignore_ascii_case(ext));
                    if !matches_ext {
                        return ignore::WalkState::Continue;
                    }
                }

                let search_name = if ignore_case {
                    name.to_lowercase()
                } else {
                    name.clone()
                };

                let matched = if let Some(ref re) = re {
                    re.is_match(&search_name)
                } else {
                    search_name.contains(&pattern)
                };

                if matched {
                    match_count.fetch_add(1, Ordering::Relaxed);
                    if !count_only {
                        println!("{}", entry.path().display());
                    }
                }

                ignore::WalkState::Continue
            })
        });
    }

    let elapsed = start.elapsed();
    let matches = match_count.load(Ordering::Relaxed);
    let files = file_count.load(Ordering::Relaxed);

    eprintln!(
        "\n{} matches found among {} entries in {:.3}s ({} threads)",
        matches, files, elapsed.as_secs_f64(), thread_count
    );
}

// ─── Index command ───────────────────────────────────────────────────

fn cmd_index(args: IndexArgs) {
    let index = build_index(&args);
    match save_index(&index) {
        Ok(()) => {
            let path = index_path_for(&args.dir);
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            eprintln!(
                "Index saved to {} ({:.1} MB)",
                path.display(),
                size as f64 / 1_048_576.0
            );
        }
        Err(e) => {
            eprintln!("Failed to save index: {}", e);
            std::process::exit(1);
        }
    }
}

// ─── Fast search (from index) ────────────────────────────────────────

fn cmd_fast(args: FastArgs) {
    let start = Instant::now();

    // Load or rebuild index
    let index = match load_index(&args.dir) {
        Some(idx) => {
            if idx.is_stale() && args.auto_reindex {
                eprintln!("Index is stale, rebuilding...");
                let new_index = build_index(&IndexArgs {
                    dir: args.dir.clone(),
                    max_age_hours: idx.max_age_secs / 3600,
                    hidden: false,
                    no_ignore: false,
                    threads: 0,
                });
                if let Err(e) = save_index(&new_index) {
                    eprintln!("Warning: failed to save updated index: {}", e);
                }
                new_index
            } else {
                if idx.is_stale() {
                    eprintln!("Warning: index is stale (use 'search index -d {}' to rebuild)", args.dir);
                }
                idx
            }
        }
        None => {
            eprintln!("No index found for '{}'. Building one now...", args.dir);
            let new_index = build_index(&IndexArgs {
                dir: args.dir.clone(),
                max_age_hours: 24,
                hidden: false,
                no_ignore: false,
                threads: 0,
            });
            if let Err(e) = save_index(&new_index) {
                eprintln!("Warning: failed to save index: {}", e);
            }
            new_index
        }
    };

    let load_elapsed = start.elapsed();

    // Prepare search pattern
    let pattern = if args.ignore_case {
        args.pattern.to_lowercase()
    } else {
        args.pattern.clone()
    };

    let re = if args.regex {
        match Regex::new(&if args.ignore_case {
            format!("(?i){}", &args.pattern)
        } else {
            args.pattern.clone()
        }) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("Invalid regex: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    // Search through index
    let search_start = Instant::now();
    let mut match_count = 0usize;

    for entry in &index.entries {
        // Type filters
        if args.dirs_only && !entry.is_dir {
            continue;
        }
        if args.files_only && entry.is_dir {
            continue;
        }

        // Size filters
        if let Some(min) = args.min_size {
            if entry.size < min {
                continue;
            }
        }
        if let Some(max) = args.max_size {
            if entry.size > max {
                continue;
            }
        }

        // Extension filter
        if let Some(ref ext) = args.ext {
            let path = Path::new(&entry.path);
            let matches_ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map_or(false, |e| e.eq_ignore_ascii_case(ext));
            if !matches_ext {
                continue;
            }
        }

        // Get file name from path
        let name = Path::new(&entry.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        let search_name = if args.ignore_case {
            name.to_lowercase()
        } else {
            name.to_string()
        };

        let matched = if let Some(ref re) = re {
            re.is_match(&search_name)
        } else {
            search_name.contains(&pattern)
        };

        if matched {
            match_count += 1;
            if !args.count {
                if entry.is_dir {
                    println!("[DIR]  {}", entry.path);
                } else {
                    println!("       {}", entry.path);
                }
            }
        }
    }

    let search_elapsed = search_start.elapsed();
    let total_elapsed = start.elapsed();

    eprintln!(
        "\n{} matches found among {} indexed entries",
        match_count,
        index.entries.len()
    );
    eprintln!(
        "Index load: {:.3}s | Search: {:.6}s | Total: {:.3}s",
        load_elapsed.as_secs_f64(),
        search_elapsed.as_secs_f64(),
        total_elapsed.as_secs_f64()
    );
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
                if !entry.file_type().map_or(false, |ft| ft.is_file()) {
                    return ignore::WalkState::Continue;
                }
                let ext_match = entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .map_or(false, |e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)));
                if !ext_match {
                    return ignore::WalkState::Continue;
                }
                let path = clean_path(&entry.path().to_string_lossy());
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    file_data.lock().unwrap().push((path, content));
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

// ─── Content index command ───────────────────────────────────────────

fn cmd_content_index(args: ContentIndexArgs) {
    let exts_str = args.ext.clone();
    let index = build_content_index(&args);
    match save_content_index(&index) {
        Ok(()) => {
            let path = content_index_path_for(&args.dir, &exts_str);
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            eprintln!(
                "Content index saved to {} ({:.1} MB)",
                path.display(),
                size as f64 / 1_048_576.0
            );
        }
        Err(e) => {
            eprintln!("Failed to save content index: {}", e);
            std::process::exit(1);
        }
    }
}

// ─── Grep command (search from content index) ────────────────────────

fn cmd_grep(args: GrepArgs) {
    let start = Instant::now();
    let exts_for_load = args.ext.clone().unwrap_or_default();

    let index = match load_content_index(&args.dir, &exts_for_load) {
        Some(idx) => {
            if idx.is_stale() && args.auto_reindex {
                eprintln!("Content index is stale, rebuilding...");
                let ext_str = idx.extensions.join(",");
                let new_idx = build_content_index(&ContentIndexArgs {
                    dir: args.dir.clone(),
                    ext: ext_str,
                    max_age_hours: idx.max_age_secs / 3600,
                    hidden: false,
                    no_ignore: false,
                    threads: 0,
                    min_token_len: 2,
                });
                let _ = save_content_index(&new_idx);
                new_idx
            } else {
                if idx.is_stale() {
                    eprintln!("Warning: content index is stale");
                }
                idx
            }
        }
        None => {
            match find_content_index_for_dir(&args.dir) {
                Some(idx) => idx,
                None => {
                    eprintln!("No content index found for '{}'. Build one first:", args.dir);
                    eprintln!("  search content-index -d {} -e cs", args.dir);
                    std::process::exit(1);
                }
            }
        }
    };

    let load_elapsed = start.elapsed();
    let search_start = Instant::now();

    // ─── Phrase search mode ─────────────────────────────────
    if args.phrase {
        let phrase = &args.pattern;
        let phrase_lower = phrase.to_lowercase();
        let phrase_tokens = tokenize(&phrase_lower, 2);

        if phrase_tokens.is_empty() {
            eprintln!("Phrase '{}' has no indexable tokens (min length 2)", phrase);
            std::process::exit(1);
        }

        // Build a whitespace-flexible regex from phrase tokens: "async void" → "async\s+void"
        let phrase_regex_pattern = phrase_tokens.iter()
            .map(|t| regex::escape(t))
            .collect::<Vec<_>>()
            .join(r"\s+");
        let phrase_re = match Regex::new(&format!("(?i){}", phrase_regex_pattern)) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to build phrase regex: {}", e);
                std::process::exit(1);
            }
        };

        eprintln!("Phrase search: '{}' → tokens: {:?} → regex: {}", phrase, phrase_tokens, phrase_regex_pattern);

        // Step 1: Find candidate files via AND search on all tokens
        let mut candidate_file_ids: Option<std::collections::HashSet<u32>> = None;
        for token in &phrase_tokens {
            if let Some(postings) = index.index.get(token.as_str()) {
                let file_ids: std::collections::HashSet<u32> = postings.iter()
                    .filter(|p| {
                        let path = &index.files[p.file_id as usize];
                        // Apply extension filter
                        if let Some(ref ext) = args.ext {
                            let m = Path::new(path).extension()
                                .and_then(|e| e.to_str())
                                .map_or(false, |e| e.eq_ignore_ascii_case(ext));
                            if !m { return false; }
                        }
                        // Apply exclude filters
                        if args.exclude_dir.iter().any(|excl| path.to_lowercase().contains(&excl.to_lowercase())) {
                            return false;
                        }
                        if args.exclude.iter().any(|excl| path.to_lowercase().contains(&excl.to_lowercase())) {
                            return false;
                        }
                        true
                    })
                    .map(|p| p.file_id)
                    .collect();
                candidate_file_ids = Some(match candidate_file_ids {
                    Some(existing) => existing.intersection(&file_ids).cloned().collect(),
                    None => file_ids,
                });
            } else {
                candidate_file_ids = Some(std::collections::HashSet::new());
                break; // Token not in index → no candidates
            }
        }

        let candidates = candidate_file_ids.unwrap_or_default();
        eprintln!("Found {} candidate files containing all tokens", candidates.len());

        // Step 2: Verify phrase in candidate files
        struct PhraseMatch {
            file_path: String,
            lines: Vec<u32>,
        }
        let mut results: Vec<PhraseMatch> = Vec::new();

        for &file_id in &candidates {
            let file_path = &index.files[file_id as usize];
            if let Ok(content) = fs::read_to_string(file_path) {
                if phrase_re.is_match(&content) {
                    // Find matching line numbers
                    let mut matching_lines = Vec::new();
                    for (line_num, line) in content.lines().enumerate() {
                        if phrase_re.is_match(line) {
                            matching_lines.push((line_num + 1) as u32);
                        }
                    }
                    if !matching_lines.is_empty() {
                        results.push(PhraseMatch {
                            file_path: file_path.clone(),
                            lines: matching_lines,
                        });
                    }
                }
            }
        }

        let search_elapsed = search_start.elapsed();
        let total_elapsed = start.elapsed();
        let match_count = results.len();
        let line_count: usize = results.iter().map(|r| r.lines.len()).sum();

        // Apply max_results
        let display_results = if args.max_results > 0 {
            &results[..results.len().min(args.max_results)]
        } else {
            &results
        };

        let ctx_before = if args.context > 0 { args.context } else { args.before };
        let ctx_after = if args.context > 0 { args.context } else { args.after };

        if !args.count {
            for result in display_results {
                if args.show_lines {
                    if let Ok(content) = fs::read_to_string(&result.file_path) {
                        let lines_vec: Vec<&str> = content.lines().collect();
                        let total_lines = lines_vec.len();
                        let mut lines_to_show: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
                        let mut match_lines_set: std::collections::HashSet<usize> = std::collections::HashSet::new();

                        for &ln in &result.lines {
                            let idx = (ln as usize).saturating_sub(1);
                            if idx < total_lines {
                                match_lines_set.insert(idx);
                                let s = idx.saturating_sub(ctx_before);
                                let e = (idx + ctx_after).min(total_lines - 1);
                                for i in s..=e { lines_to_show.insert(i); }
                            }
                        }

                        let mut prev: Option<usize> = None;
                        for &idx in &lines_to_show {
                            if let Some(p) = prev { if idx > p + 1 { println!("--"); } }
                            let marker = if match_lines_set.contains(&idx) { ">" } else { " " };
                            println!("{}{}:{}: {}", marker, result.file_path, idx + 1, lines_vec[idx]);
                            prev = Some(idx);
                        }
                        if !lines_to_show.is_empty() { println!(); }
                    }
                } else {
                    println!(
                        "{} ({} matches, lines: {})",
                        result.file_path,
                        result.lines.len(),
                        result.lines.iter().take(10).map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
                    );
                }
            }
        }

        eprintln!(
            "\n{} files, {} lines matching phrase '{}' (candidates: {}, index: {} files)",
            match_count, line_count, phrase, candidates.len(), index.files.len()
        );
        eprintln!(
            "Index load: {:.3}s | Search+Verify: {:.6}s | Total: {:.3}s",
            load_elapsed.as_secs_f64(), search_elapsed.as_secs_f64(), total_elapsed.as_secs_f64()
        );
        return;
    }

    // ─── Normal token search ────────────────────────────────

    // Parse comma-separated terms
    let raw_terms: Vec<String> = args.pattern
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    // If regex mode, expand each pattern by matching against all index keys
    let terms: Vec<String> = if args.regex {
        let mut expanded = Vec::new();
        for pat in &raw_terms {
            match Regex::new(&format!("(?i)^{}$", pat)) {
                Ok(re) => {
                    let matching: Vec<String> = index.index.keys()
                        .filter(|k| re.is_match(k))
                        .cloned()
                        .collect();
                    if matching.is_empty() {
                        eprintln!("Warning: regex '{}' matched 0 tokens", pat);
                    } else {
                        eprintln!("Regex '{}' matched {} tokens", pat, matching.len());
                    }
                    expanded.extend(matching);
                }
                Err(e) => {
                    eprintln!("Invalid regex '{}': {}", pat, e);
                    std::process::exit(1);
                }
            }
        }
        expanded
    } else {
        raw_terms.clone()
    };

    let total_docs = index.files.len() as f64;
    let mode_str = if args.regex { "REGEX" } else if args.all { "AND" } else { "OR" };

    // Collect per-file scores across all terms
    struct FileScore {
        file_path: String,
        lines: Vec<u32>,
        tf_idf: f64,
        occurrences: usize,
        terms_matched: usize,
    }

    let mut file_scores: HashMap<u32, FileScore> = HashMap::new();
    let term_count_for_all = if args.regex { raw_terms.len() } else { terms.len() };

    for term in &terms {
        if let Some(postings) = index.index.get(term.as_str()) {
            let doc_freq = postings.len() as f64;
            let idf = (total_docs / doc_freq).ln();

            for posting in postings {
                let file_path = &index.files[posting.file_id as usize];

                // Extension filter
                if let Some(ref ext) = args.ext {
                    let matches = Path::new(file_path)
                        .extension()
                        .and_then(|e| e.to_str())
                        .map_or(false, |e| e.eq_ignore_ascii_case(ext));
                    if !matches {
                        continue;
                    }
                }

                // Exclude dir filter
                if args.exclude_dir.iter().any(|excl| {
                    let excl_lower = excl.to_lowercase();
                    file_path.to_lowercase().contains(&excl_lower)
                }) {
                    continue;
                }

                // Exclude pattern filter
                if args.exclude.iter().any(|excl| {
                    let excl_lower = excl.to_lowercase();
                    file_path.to_lowercase().contains(&excl_lower)
                }) {
                    continue;
                }

                let occurrences = posting.lines.len();
                let file_total = if (posting.file_id as usize) < index.file_token_counts.len() {
                    index.file_token_counts[posting.file_id as usize] as f64
                } else {
                    1.0
                };
                let tf = occurrences as f64 / file_total;
                let tf_idf = tf * idf;

                let entry = file_scores.entry(posting.file_id).or_insert(FileScore {
                    file_path: file_path.clone(),
                    lines: Vec::new(),
                    tf_idf: 0.0,
                    occurrences: 0,
                    terms_matched: 0,
                });
                entry.tf_idf += tf_idf; // sum scores across terms
                entry.occurrences += occurrences;
                entry.lines.extend_from_slice(&posting.lines);
                entry.terms_matched += 1;
            }
        }
    }

    // Filter by AND mode if needed (for regex, AND applies to raw patterns, not expanded tokens)
    let mut results: Vec<FileScore> = file_scores
        .into_values()
        .filter(|fs| !args.all || fs.terms_matched >= term_count_for_all)
        .collect();

    // Sort lines within each result and deduplicate
    for result in &mut results {
        result.lines.sort();
        result.lines.dedup();
    }

    // Sort by TF-IDF score descending
    results.sort_by(|a, b| b.tf_idf.partial_cmp(&a.tf_idf).unwrap_or(std::cmp::Ordering::Equal));

    let match_count = results.len();
    let line_count: usize = results.iter().map(|r| r.lines.len()).sum();

    // Apply max_results limit
    let display_results = if args.max_results > 0 {
        &results[..results.len().min(args.max_results)]
    } else {
        &results
    };

    // Calculate context: -C overrides -B/-A if set
    let ctx_before = if args.context > 0 { args.context } else { args.before };
    let ctx_after = if args.context > 0 { args.context } else { args.after };

    if !args.count {
        for result in display_results {
            if args.show_lines {
                if let Ok(content) = fs::read_to_string(&result.file_path) {
                    let lines_vec: Vec<&str> = content.lines().collect();
                    let total_lines = lines_vec.len();

                    // Build set of all lines to display (match lines + context)
                    let mut lines_to_show: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
                    let mut match_lines: std::collections::HashSet<usize> = std::collections::HashSet::new();

                    for &line_num in &result.lines {
                        let idx = (line_num as usize).saturating_sub(1);
                        if idx < total_lines {
                            match_lines.insert(idx);
                            // Add context before
                            let start = idx.saturating_sub(ctx_before);
                            // Add context after
                            let end = (idx + ctx_after).min(total_lines - 1);
                            for i in start..=end {
                                lines_to_show.insert(i);
                            }
                        }
                    }

                    let mut prev_idx: Option<usize> = None;
                    for &idx in &lines_to_show {
                        // Print separator between non-contiguous blocks
                        if let Some(prev) = prev_idx {
                            if idx > prev + 1 {
                                println!("--");
                            }
                        }
                        let marker = if match_lines.contains(&idx) { ">" } else { " " };
                        println!("{}{}:{}: {}", marker, result.file_path, idx + 1, lines_vec[idx]);
                        prev_idx = Some(idx);
                    }
                    if !lines_to_show.is_empty() {
                        println!(); // blank line between files
                    }
                }
            } else {
                println!(
                    "[{:.4}] {} ({} occurrences, {}/{} terms, lines: {})",
                    result.tf_idf,
                    result.file_path,
                    result.occurrences,
                    result.terms_matched,
                    terms.len(),
                    result.lines.iter().take(10).map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
                );
            }
        }
    }

    let search_elapsed = search_start.elapsed();
    let total_elapsed = start.elapsed();

    eprintln!(
        "\n{} files, {} occurrences matching {} terms [{}]: '{}' (index: {} files, {} unique tokens)",
        match_count, line_count, terms.len(), mode_str, args.pattern, index.files.len(), index.index.len()
    );
    eprintln!(
        "Index load: {:.3}s | Search+Rank: {:.6}s | Total: {:.3}s",
        load_elapsed.as_secs_f64(), search_elapsed.as_secs_f64(), total_elapsed.as_secs_f64()
    );
}

// ─── Info command ────────────────────────────────────────────────────

fn cmd_info() {
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
            if let Ok(data) = fs::read(&path) {
                if let Ok(index) = bincode::deserialize::<FileIndex>(&data) {
                    found = true;
                    let age_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        - index.created_at;
                    let age_hours = age_secs as f64 / 3600.0;
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let stale = if index.is_stale() { " [STALE]" } else { "" };
                    println!(
                        "  [FILE] {} -- {} entries, {:.1} MB, {:.1}h ago{}",
                        index.root, index.entries.len(),
                        size as f64 / 1_048_576.0, age_hours, stale
                    );
                }
            }
        } else if ext == Some("cidx") {
            if let Ok(data) = fs::read(&path) {
                if let Ok(index) = bincode::deserialize::<ContentIndex>(&data) {
                    found = true;
                    let age_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        - index.created_at;
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
                if let Ok(data) = fs::read(&path) {
                    if let Ok(index) = bincode::deserialize::<FileIndex>(&data) {
                        let age_secs = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
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
                }
            } else if ext == Some("cidx") {
                if let Ok(data) = fs::read(&path) {
                    if let Ok(index) = bincode::deserialize::<ContentIndex>(&data) {
                        let age_secs = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
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
                }
            }
        }
    }

    serde_json::json!({
        "directory": dir.display().to_string(),
        "indexes": indexes,
    })
}

// ─── Serve command ───────────────────────────────────────────────────

fn cmd_serve(args: ServeArgs) {
    let dir_str = args.dir.clone();
    let ext_str = args.ext.clone();
    let extensions: Vec<String> = ext_str.split(',').map(|s| s.trim().to_lowercase()).collect();
    let exts_for_load = extensions.join(",");

    // Initialize tracing subscriber for structured logging to stderr.
    // MCP protocol uses stdout for JSON-RPC, so all logs MUST go to stderr.
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

    // Load or build content index
    let start = Instant::now();
    let index = match load_content_index(&dir_str, &exts_for_load) {
        Some(idx) => {
            info!(files = idx.files.len(), tokens = idx.index.len(), "Loaded content index");
            idx
        }
        None => {
            match find_content_index_for_dir(&dir_str) {
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
                        min_token_len: 2,
                    });
                    if let Err(e) = save_content_index(&new_idx) {
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

    // Wrap in Arc<RwLock> for thread safety
    let index = if args.watch {
        // Build forward index for watch mode
        let watch_idx = mcp::watcher::build_watch_index_from(index);
        Arc::new(RwLock::new(watch_idx))
    } else {
        Arc::new(RwLock::new(index))
    };

    // Load or build definition index if --definitions
    let def_index = if args.definitions {
        let def_start = Instant::now();
        let def_exts = "cs,sql";

        // Try to load existing definition index from disk first
        let def_idx = match definitions::load_definition_index(&dir_str, def_exts) {
            Some(idx) => {
                info!(definitions = idx.definitions.len(), files = idx.files.len(),
                    "Loaded definition index from disk");
                idx
            }
            None => {
                match definitions::find_definition_index_for_dir(&dir_str) {
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
                        if let Err(e) = definitions::save_definition_index(&new_idx) {
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
        let watch_dir = fs::canonicalize(&dir_str)
            .unwrap_or_else(|_| PathBuf::from(&dir_str));
        if let Err(e) = mcp::watcher::start_watcher(
            Arc::clone(&index),
            def_index.as_ref().map(Arc::clone),
            watch_dir,
            extensions,
            args.debounce_ms,
            args.bulk_threshold,
        ) {
            warn!(error = %e, "Failed to start file watcher");
        }
    }

    // Start MCP server event loop
    mcp::server::run_server(index, def_index, dir_str, exts_for_load);
}

fn cmd_def_index(args: definitions::DefIndexArgs) {
    let index = definitions::build_definition_index(&args);
    match definitions::save_definition_index(&index) {
        Ok(()) => {
            eprintln!("[def-index] Done! {} definitions from {} files",
                index.definitions.len(), index.files.len());
        }
        Err(e) => {
            eprintln!("[def-index] Error saving index: {}", e);
            std::process::exit(1);
        }
    }
}

// ─── Main ────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Find(args) => cmd_find(args),
        Commands::Index(args) => cmd_index(args),
        Commands::Fast(args) => cmd_fast(args),
        Commands::Info => cmd_info(),
        Commands::ContentIndex(args) => cmd_content_index(args),
        Commands::Grep(args) => cmd_grep(args),
        Commands::Serve(args) => cmd_serve(args),
        Commands::DefIndex(args) => cmd_def_index(args),
    }
}


// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ─── clean_path tests ────────────────────────────────────────

    #[test]
    fn test_clean_path_strips_prefix() {
        assert_eq!(clean_path(r"\\?\C:\Windows\notepad.exe"), r"C:\Windows\notepad.exe");
    }

    #[test]
    fn test_clean_path_no_prefix() {
        assert_eq!(clean_path(r"C:\Windows\notepad.exe"), r"C:\Windows\notepad.exe");
    }

    #[test]
    fn test_clean_path_unix_style() {
        assert_eq!(clean_path("/usr/bin/ls"), "/usr/bin/ls");
    }

    // ─── tokenize tests ─────────────────────────────────────────

    #[test]
    fn test_tokenize_basic() {
        let tokens = tokenize("Hello World", 2);
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_tokenize_code() {
        let tokens = tokenize("private readonly HttpClient _client;", 2);
        assert_eq!(tokens, vec!["private", "readonly", "httpclient", "_client"]);
    }

    #[test]
    fn test_tokenize_min_length() {
        let tokens = tokenize("a bb ccc", 2);
        assert_eq!(tokens, vec!["bb", "ccc"]);
    }

    #[test]
    fn test_tokenize_with_numbers() {
        let tokens = tokenize("var x2 = getValue(item3);", 2);
        assert!(tokens.contains(&"x2".to_string()));
        assert!(tokens.contains(&"getvalue".to_string()));
        assert!(tokens.contains(&"item3".to_string()));
    }

    #[test]
    fn test_tokenize_underscores() {
        let tokens = tokenize("my_variable = some_func()", 2);
        assert!(tokens.contains(&"my_variable".to_string()));
        assert!(tokens.contains(&"some_func".to_string()));
    }

    #[test]
    fn test_tokenize_empty_string() {
        let tokens = tokenize("", 2);
        assert!(tokens.is_empty());
    }

    // ─── FileIndex staleness tests ──────────────────────────────

    #[test]
    fn test_file_index_not_stale() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let index = FileIndex {
            root: ".".to_string(),
            created_at: now,
            max_age_secs: 3600,
            entries: vec![],
        };
        assert!(!index.is_stale());
    }

    #[test]
    fn test_file_index_stale() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let index = FileIndex {
            root: ".".to_string(),
            created_at: now - 7200, // 2 hours ago
            max_age_secs: 3600,     // 1 hour max
            entries: vec![],
        };
        assert!(index.is_stale());
    }

    // ─── ContentIndex staleness tests ───────────────────────────

    #[test]
    fn test_content_index_not_stale() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let index = ContentIndex {
            root: ".".to_string(),
            created_at: now,
            max_age_secs: 3600,
            files: vec![],
            index: HashMap::new(),
            total_tokens: 0,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![],
            forward: None,
            path_to_id: None,
        };
        assert!(!index.is_stale());
    }

    #[test]
    fn test_content_index_stale() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let index = ContentIndex {
            root: ".".to_string(),
            created_at: now - 7200,
            max_age_secs: 3600,
            files: vec![],
            index: HashMap::new(),
            total_tokens: 0,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![],
            forward: None,
            path_to_id: None,
        };
        assert!(index.is_stale());
    }

    // ─── Serialization roundtrip tests ──────────────────────────

    #[test]
    fn test_file_index_serialization_roundtrip() {
        let index = FileIndex {
            root: "C:\\test".to_string(),
            created_at: 1000000,
            max_age_secs: 3600,
            entries: vec![
                FileEntry {
                    path: "C:\\test\\file1.txt".to_string(),
                    size: 1024,
                    modified: 999999,
                    is_dir: false,
                },
                FileEntry {
                    path: "C:\\test\\subdir".to_string(),
                    size: 0,
                    modified: 999998,
                    is_dir: true,
                },
            ],
        };
        let encoded = bincode::serialize(&index).unwrap();
        let decoded: FileIndex = bincode::deserialize(&encoded).unwrap();
        assert_eq!(decoded.root, "C:\\test");
        assert_eq!(decoded.entries.len(), 2);
        assert_eq!(decoded.entries[0].path, "C:\\test\\file1.txt");
        assert_eq!(decoded.entries[0].size, 1024);
        assert!(!decoded.entries[0].is_dir);
        assert!(decoded.entries[1].is_dir);
    }

    #[test]
    fn test_content_index_serialization_roundtrip() {
        let mut idx = HashMap::new();
        idx.insert(
            "httpclient".to_string(),
            vec![Posting {
                file_id: 0,
                lines: vec![5, 12, 30],
            }],
        );
        let index = ContentIndex {
            root: "C:\\test".to_string(),
            created_at: 1000000,
            max_age_secs: 3600,
            files: vec!["C:\\test\\Program.cs".to_string()],
            index: idx,
            total_tokens: 100,
            extensions: vec!["cs".to_string()],
            file_token_counts: vec![50],
            forward: None,
            path_to_id: None,
        };
        let encoded = bincode::serialize(&index).unwrap();
        let decoded: ContentIndex = bincode::deserialize(&encoded).unwrap();
        assert_eq!(decoded.root, "C:\\test");
        assert_eq!(decoded.files.len(), 1);
        assert_eq!(decoded.total_tokens, 100);
        assert_eq!(decoded.file_token_counts, vec![50]);
        let postings = decoded.index.get("httpclient").unwrap();
        assert_eq!(postings.len(), 1);
        assert_eq!(postings[0].file_id, 0);
        assert_eq!(postings[0].lines, vec![5, 12, 30]);
    }

    // ─── TF-IDF scoring tests ───────────────────────────────────

    #[test]
    fn test_tf_idf_more_relevant_file_scores_higher() {
        // File A: small file, HttpClient is 50% of tokens → high TF
        // File B: big file, HttpClient is 1% of tokens → low TF
        let total_docs = 1000.0_f64;
        let doc_freq = 100.0_f64;
        let idf = (total_docs / doc_freq).ln();

        let tf_a = 5.0 / 10.0;  // 50% of file A
        let tf_b = 5.0 / 500.0; // 1% of file B

        let score_a = tf_a * idf;
        let score_b = tf_b * idf;

        assert!(score_a > score_b, "Smaller, more focused file should rank higher");
        assert!(score_a > 0.0);
        assert!(score_b > 0.0);
    }

    #[test]
    fn test_tf_idf_rare_term_scores_higher() {
        // Same TF, but term A appears in 10 docs, term B in 900 docs
        let total_docs = 1000.0_f64;
        let tf = 0.1;

        let idf_rare = (total_docs / 10.0).ln();
        let idf_common = (total_docs / 900.0).ln();

        let score_rare = tf * idf_rare;
        let score_common = tf * idf_common;

        assert!(score_rare > score_common, "Rare term should score higher");
    }

    // ─── Integration test: build and search content index ────────

    #[test]
    fn test_build_and_search_content_index() {
        let dir = std::env::temp_dir().join("search_test_content_idx");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Create test files
        let mut f1 = fs::File::create(dir.join("factory.cs")).unwrap();
        writeln!(f1, "using System;").unwrap();
        writeln!(f1, "class HttpClientFactory {{").unwrap();
        writeln!(f1, "    HttpClient Create() {{ return new HttpClient(); }}").unwrap();
        writeln!(f1, "}}").unwrap();

        let mut f2 = fs::File::create(dir.join("program.cs")).unwrap();
        writeln!(f2, "using System;").unwrap();
        writeln!(f2, "using System.Net;").unwrap();
        writeln!(f2, "using System.IO;").unwrap();
        writeln!(f2, "using System.Linq;").unwrap();
        writeln!(f2, "class Program {{").unwrap();
        writeln!(f2, "    static void Main() {{").unwrap();
        writeln!(f2, "        var client = new HttpClient();").unwrap();
        writeln!(f2, "        Console.WriteLine(client.GetAsync(\"/api\"));").unwrap();
        writeln!(f2, "    }}").unwrap();
        writeln!(f2, "}}").unwrap();

        let args = ContentIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
            min_token_len: 2,
        };

        let index = build_content_index(&args);

        assert_eq!(index.files.len(), 2);
        assert!(index.total_tokens > 0);
        assert!(index.file_token_counts.len() == 2);

        // Search for "httpclient"
        let postings = index.index.get("httpclient");
        assert!(postings.is_some(), "httpclient should be in index");

        let postings = postings.unwrap();
        assert!(postings.len() == 2, "httpclient should appear in both files");

        // factory.cs should have more occurrences relative to file size (higher TF)
        let factory_posting = postings.iter().find(|p| {
            index.files[p.file_id as usize].contains("factory")
        });
        let program_posting = postings.iter().find(|p| {
            index.files[p.file_id as usize].contains("program")
        });

        assert!(factory_posting.is_some());
        assert!(program_posting.is_some());

        let factory_id = factory_posting.unwrap().file_id as usize;
        let program_id = program_posting.unwrap().file_id as usize;

        let tf_factory = factory_posting.unwrap().lines.len() as f64
            / index.file_token_counts[factory_id] as f64;
        let tf_program = program_posting.unwrap().lines.len() as f64
            / index.file_token_counts[program_id] as f64;

        assert!(
            tf_factory > tf_program,
            "factory.cs should have higher TF ({tf_factory}) than program.cs ({tf_program})"
        );

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    // ─── Integration test: build file index ─────────────────────

    #[test]
    fn test_build_file_index() {
        let dir = std::env::temp_dir().join("search_test_file_idx");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("subdir")).unwrap();

        fs::write(dir.join("file1.txt"), "hello").unwrap();
        fs::write(dir.join("file2.rs"), "fn main() {}").unwrap();
        fs::write(dir.join("subdir").join("file3.txt"), "world").unwrap();

        let args = IndexArgs {
            dir: dir.to_string_lossy().to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
        };

        let index = build_index(&args);

        // Should have at least 3 files + 2 directories (root + subdir)
        assert!(index.entries.len() >= 4, "Expected at least 4 entries, got {}", index.entries.len());

        let file_paths: Vec<&str> = index.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(
            file_paths.iter().any(|p| p.contains("file1.txt")),
            "file1.txt should be in index"
        );
        assert!(
            file_paths.iter().any(|p| p.contains("file2.rs")),
            "file2.rs should be in index"
        );
        assert!(
            file_paths.iter().any(|p| p.contains("file3.txt")),
            "file3.txt should be in index"
        );

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    // ─── Multi-term search tests ────────────────────────────────

    #[test]
    fn test_multi_term_or_search() {
        let dir = std::env::temp_dir().join("search_test_multi_or");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut f1 = fs::File::create(dir.join("both.cs")).unwrap();
        writeln!(f1, "class Foo {{ HttpClient client; ILogger logger; }}").unwrap();

        let mut f2 = fs::File::create(dir.join("only_client.cs")).unwrap();
        writeln!(f2, "class Bar {{ HttpClient client; }}").unwrap();

        let mut f3 = fs::File::create(dir.join("only_logger.cs")).unwrap();
        writeln!(f3, "class Baz {{ ILogger logger; }}").unwrap();

        let mut f4 = fs::File::create(dir.join("neither.cs")).unwrap();
        writeln!(f4, "class Empty {{ int x; }}").unwrap();

        let args = ContentIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
            min_token_len: 2,
        };
        let index = build_content_index(&args);

        // OR: files with "httpclient" OR "ilogger"
        let term1_postings = index.index.get("httpclient");
        let term2_postings = index.index.get("ilogger");

        assert!(term1_postings.is_some());
        assert!(term2_postings.is_some());

        // Collect all file_ids from both terms (union = OR)
        let mut or_files: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for p in term1_postings.unwrap() { or_files.insert(p.file_id); }
        for p in term2_postings.unwrap() { or_files.insert(p.file_id); }

        // both.cs, only_client.cs, only_logger.cs = 3 files
        assert_eq!(or_files.len(), 3, "OR should match 3 files");

        // AND: intersection
        let t1_files: std::collections::HashSet<u32> = term1_postings.unwrap().iter().map(|p| p.file_id).collect();
        let t2_files: std::collections::HashSet<u32> = term2_postings.unwrap().iter().map(|p| p.file_id).collect();
        let and_files: Vec<u32> = t1_files.intersection(&t2_files).cloned().collect();

        // Only both.cs
        assert_eq!(and_files.len(), 1, "AND should match 1 file");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_multi_term_and_search() {
        let dir = std::env::temp_dir().join("search_test_multi_and");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut f1 = fs::File::create(dir.join("all_three.cs")).unwrap();
        writeln!(f1, "HttpClient Task ILogger").unwrap();

        let mut f2 = fs::File::create(dir.join("two_of_three.cs")).unwrap();
        writeln!(f2, "HttpClient Task SomeOther").unwrap();

        let args = ContentIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
            min_token_len: 2,
        };
        let index = build_content_index(&args);

        // Check all three terms exist
        let terms = ["httpclient", "task", "ilogger"];
        for term in &terms {
            assert!(index.index.contains_key(*term), "Term '{}' should be in index", term);
        }

        // AND: only all_three.cs should have all 3 terms
        let file_sets: Vec<std::collections::HashSet<u32>> = terms.iter()
            .map(|t| index.index.get(*t).unwrap().iter().map(|p| p.file_id).collect())
            .collect();

        let intersection = file_sets.iter().skip(1).fold(file_sets[0].clone(), |acc, s| {
            acc.intersection(s).cloned().collect()
        });

        assert_eq!(intersection.len(), 1, "Only 1 file should contain all 3 terms");

        let _ = fs::remove_dir_all(&dir);
    }

    // ─── Regex search tests ─────────────────────────────────────

    #[test]
    fn test_regex_token_matching() {
        let dir = std::env::temp_dir().join("search_test_regex");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut f1 = fs::File::create(dir.join("caches.cs")).unwrap();
        writeln!(f1, "ITenantCache IUserCache ISessionCache INotAMatch").unwrap();

        let args = ContentIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
            min_token_len: 2,
        };
        let index = build_content_index(&args);

        // Regex "i.*cache" should match itenantcache, iusercache, isessioncache
        let re = Regex::new("(?i)^i.*cache$").unwrap();
        let matching_tokens: Vec<&String> = index.index.keys()
            .filter(|k| re.is_match(k))
            .collect();

        assert!(matching_tokens.len() >= 3,
            "Should match at least 3 cache tokens, got {}: {:?}", matching_tokens.len(), matching_tokens);

        // "inotamatch" should NOT match the cache regex
        assert!(
            !matching_tokens.contains(&&"inotamatch".to_string()),
            "inotamatch should not match i.*cache pattern"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_regex_no_match() {
        let dir = std::env::temp_dir().join("search_test_regex_no");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut f1 = fs::File::create(dir.join("simple.cs")).unwrap();
        writeln!(f1, "class Foo {{ int x; }}").unwrap();

        let args = ContentIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
            min_token_len: 2,
        };
        let index = build_content_index(&args);

        let re = Regex::new("(?i)^zzzznonexistent$").unwrap();
        let matching: Vec<&String> = index.index.keys()
            .filter(|k| re.is_match(k))
            .collect();

        assert_eq!(matching.len(), 0, "Non-existent pattern should match 0 tokens");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_regex_matches_partial_tokens() {
        let dir = std::env::temp_dir().join("search_test_regex_partial");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut f1 = fs::File::create(dir.join("async.cs")).unwrap();
        writeln!(f1, "GetAsync PostAsync SendAsync SyncMethod").unwrap();

        let args = ContentIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
            min_token_len: 2,
        };
        let index = build_content_index(&args);

        // Pattern ".*async" should match getasync, postasync, sendasync
        let re = Regex::new("(?i)^.*async$").unwrap();
        let matching: Vec<&String> = index.index.keys()
            .filter(|k| re.is_match(k))
            .collect();

        assert!(matching.len() >= 3, "Should match at least 3 async tokens, got {}: {:?}", matching.len(), matching);
        assert!(
            !matching.contains(&&"syncmethod".to_string()),
            "syncmethod should not match .*async$ pattern"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // ─── Exclude filter tests ───────────────────────────────────

    #[test]
    fn test_exclude_dir_filters_paths() {
        let dir = std::env::temp_dir().join("search_excl_dir");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("zzztests")).unwrap();
        fs::create_dir_all(dir.join("zzztests").join("zzzE2E")).unwrap();

        let mut f1 = fs::File::create(dir.join("src").join("main.cs")).unwrap();
        writeln!(f1, "class Main {{ HttpClient client; }}").unwrap();

        let mut f2 = fs::File::create(dir.join("zzztests").join("test1.cs")).unwrap();
        writeln!(f2, "class Test1 {{ HttpClient client; }}").unwrap();

        let mut f3 = fs::File::create(dir.join("zzztests").join("zzzE2E").join("e2e.cs")).unwrap();
        writeln!(f3, "class E2ETest {{ HttpClient client; }}").unwrap();

        let args = ContentIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
            min_token_len: 2,
        };
        let index = build_content_index(&args);

        assert_eq!(index.files.len(), 3, "Should index 3 files");

        // Simulate exclude_dir filtering (using unique name to avoid matching temp path)
        let exclude_dirs = vec!["zzztests".to_string()];
        let postings = index.index.get("httpclient").unwrap();
        let filtered: Vec<_> = postings.iter()
            .filter(|p| {
                let path = &index.files[p.file_id as usize];
                !exclude_dirs.iter().any(|excl| path.to_lowercase().contains(&excl.to_lowercase()))
            })
            .collect();

        // Only src/main.cs should remain (zzztests/ and zzztests/zzzE2E/ excluded)
        assert_eq!(filtered.len(), 1, "After excluding 'zzztests' dir, should have 1 file");
        assert!(
            index.files[filtered[0].file_id as usize].contains("main.cs"),
            "Remaining file should be main.cs"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_exclude_pattern_filters_files() {
        let dir = std::env::temp_dir().join("search_test_exclude_pat");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut f1 = fs::File::create(dir.join("Service.cs")).unwrap();
        writeln!(f1, "class Service {{ HttpClient c; }}").unwrap();

        let mut f2 = fs::File::create(dir.join("ServiceMock.cs")).unwrap();
        writeln!(f2, "class ServiceMock {{ HttpClient c; }}").unwrap();

        let mut f3 = fs::File::create(dir.join("ServiceTests.cs")).unwrap();
        writeln!(f3, "class ServiceTests {{ HttpClient c; }}").unwrap();

        let args = ContentIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
            min_token_len: 2,
        };
        let index = build_content_index(&args);

        let postings = index.index.get("httpclient").unwrap();

        // Exclude Mock and Tests
        let excludes = vec!["mock".to_string(), "tests".to_string()];
        let filtered: Vec<_> = postings.iter()
            .filter(|p| {
                let path = &index.files[p.file_id as usize];
                !excludes.iter().any(|excl| path.to_lowercase().contains(&excl.to_lowercase()))
            })
            .collect();

        assert_eq!(filtered.len(), 1, "After excluding Mock and Tests, should have 1 file");
        assert!(
            index.files[filtered[0].file_id as usize].contains("Service.cs")
                && !index.files[filtered[0].file_id as usize].contains("Mock"),
            "Remaining file should be Service.cs (not Mock or Tests)"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // ─── Context lines tests ────────────────────────────────────

    #[test]
    fn test_context_lines_calculation() {
        // Test the context window logic directly
        let total_lines = 10;
        let match_line: usize = 5; // 0-indexed
        let ctx = 2;

        let start = match_line.saturating_sub(ctx);
        let end = (match_line + ctx).min(total_lines - 1);

        assert_eq!(start, 3, "Context should start at line 3 (2 before line 5)");
        assert_eq!(end, 7, "Context should end at line 7 (2 after line 5)");
    }

    #[test]
    fn test_context_lines_at_file_boundaries() {
        // Match at line 1 (index 0) with context 3 → should not go below 0
        let match_line: usize = 0;
        let ctx = 3;
        let total_lines = 10;

        let start = match_line.saturating_sub(ctx);
        let end = (match_line + ctx).min(total_lines - 1);

        assert_eq!(start, 0, "Context should not go below 0");
        assert_eq!(end, 3, "Context should extend to line 3");

        // Match at last line with context 3 → should not exceed total
        let match_line2: usize = 9;
        let start2 = match_line2.saturating_sub(ctx);
        let end2 = (match_line2 + ctx).min(total_lines - 1);

        assert_eq!(start2, 6, "Context before should be line 6");
        assert_eq!(end2, 9, "Context should not exceed total_lines - 1");
    }

    #[test]
    fn test_context_merges_overlapping_ranges() {
        // Two matches close together should merge context
        let match_lines = vec![4usize, 6usize]; // 0-indexed
        let ctx = 2;
        let total_lines = 15;

        let mut lines_to_show: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        for &m in &match_lines {
            let start = m.saturating_sub(ctx);
            let end = (m + ctx).min(total_lines - 1);
            for i in start..=end {
                lines_to_show.insert(i);
            }
        }

        // Lines 2-8 should be in the set (merged ranges)
        let result: Vec<usize> = lines_to_show.into_iter().collect();
        assert_eq!(result, vec![2, 3, 4, 5, 6, 7, 8], "Overlapping contexts should merge");
    }

    // ─── Phrase search tests ────────────────────────────────────

    #[test]
    fn test_phrase_search_finds_exact_phrase() {
        let dir = std::env::temp_dir().join("search_phrase_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut f1 = fs::File::create(dir.join("has_phrase.cs")).unwrap();
        writeln!(f1, "using System;").unwrap();
        writeln!(f1, "var client = new HttpClient();").unwrap();
        writeln!(f1, "client.GetAsync(\"/api\");").unwrap();

        let mut f2 = fs::File::create(dir.join("has_words_but_not_phrase.cs")).unwrap();
        writeln!(f2, "// HttpClient is useful").unwrap();
        writeln!(f2, "// but we use new patterns here").unwrap();
        writeln!(f2, "var x = new Something();").unwrap();

        let mut f3 = fs::File::create(dir.join("no_match.cs")).unwrap();
        writeln!(f3, "class Empty {{ }}").unwrap();

        let args = ContentIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
            min_token_len: 2,
        };
        let index = build_content_index(&args);

        // Simulate phrase search: tokenize, AND search, then verify
        let phrase = "new HttpClient";
        let phrase_lower = phrase.to_lowercase();
        let phrase_tokens = tokenize(&phrase_lower, 2);

        assert_eq!(phrase_tokens, vec!["new", "httpclient"]);

        // AND search: find files with both "new" AND "httpclient"
        let mut candidate_ids: Option<std::collections::HashSet<u32>> = None;
        for token in &phrase_tokens {
            if let Some(postings) = index.index.get(token.as_str()) {
                let ids: std::collections::HashSet<u32> = postings.iter().map(|p| p.file_id).collect();
                candidate_ids = Some(match candidate_ids {
                    Some(existing) => existing.intersection(&ids).cloned().collect(),
                    None => ids,
                });
            }
        }
        let candidates = candidate_ids.unwrap_or_default();
        // Both files 1 and 2 have "new" and "httpclient" (but not as adjacent phrase in file 2)
        assert!(candidates.len() >= 1, "Should find at least 1 candidate");

        // Verify: only file 1 has the exact phrase
        let mut verified = Vec::new();
        for &fid in &candidates {
            let path = &index.files[fid as usize];
            if let Ok(content) = fs::read_to_string(path) {
                if content.to_lowercase().contains(&phrase_lower) {
                    verified.push(fid);
                }
            }
        }

        assert_eq!(verified.len(), 1, "Only 1 file should contain exact phrase 'new HttpClient'");
        assert!(
            index.files[verified[0] as usize].contains("has_phrase"),
            "The verified file should be has_phrase.cs"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_phrase_search_no_match() {
        let dir = std::env::temp_dir().join("search_phrase_nomatch");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut f1 = fs::File::create(dir.join("file.cs")).unwrap();
        writeln!(f1, "class Foo {{ int x; string y; }}").unwrap();

        let args = ContentIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
            min_token_len: 2,
        };
        let index = build_content_index(&args);

        let phrase = "new HttpClient";
        let phrase_lower = phrase.to_lowercase();
        let phrase_tokens = tokenize(&phrase_lower, 2);

        // "new" and "httpclient" are not in the index for this file
        let has_all = phrase_tokens.iter().all(|t| index.index.contains_key(t.as_str()));
        assert!(!has_all, "Not all phrase tokens should exist in index");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_phrase_search_case_insensitive() {
        let dir = std::env::temp_dir().join("search_phrase_case");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut f1 = fs::File::create(dir.join("mixed.cs")).unwrap();
        writeln!(f1, "var c = New HTTPCLIENT();").unwrap();

        let args = ContentIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs".to_string(),
            max_age_hours: 24,
            hidden: false,
            no_ignore: true,
            threads: 1,
            min_token_len: 2,
        };
        let index = build_content_index(&args);

        let phrase = "new HttpClient";
        let phrase_lower = phrase.to_lowercase();

        // Verify case-insensitive match
        let fid = 0u32;
        let content = fs::read_to_string(&index.files[fid as usize]).unwrap();
        assert!(
            content.to_lowercase().contains(&phrase_lower),
            "Case-insensitive phrase match should work"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
