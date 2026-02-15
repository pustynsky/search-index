# Storage Model

## Index File Layout

All indexes are stored under a platform-specific data directory:

| OS      | Path                                          |
| ------- | --------------------------------------------- |
| Windows | `%LOCALAPPDATA%\search-index\`                |
| macOS   | `~/Library/Application Support/search-index/` |
| Linux   | `~/.local/share/search-index/`                |

```
search-index/
├── a1b2c3d4e5f67890.idx      ← FileIndex for directory A
├── f0e1d2c3b4a59678.cidx     ← ContentIndex for directory B + extensions "cs"
├── 1234567890abcdef.cidx     ← ContentIndex for directory B + extensions "cs,sql"
├── abcdef1234567890.didx     ← DefinitionIndex for directory B + extensions "cs,sql"
└── ...
```

### File Naming Scheme

Each index file is named by a 64-bit hash of its identity:

```rust
// FileIndex: hash of canonical directory path
let mut hasher = DefaultHasher::new();
canonical_dir.hash(&mut hasher);
let filename = format!("{:016x}.idx", hasher.finish());

// ContentIndex: hash of canonical dir + extension string
canonical_dir.hash(&mut hasher);
extensions_string.hash(&mut hasher);
let filename = format!("{:016x}.cidx", hasher.finish());

// DefinitionIndex: same scheme as ContentIndex
let filename = format!("{:016x}.didx", hasher.finish());
```

**Implication:** Indexing the same directory with different extension sets produces different files. `search content-index -d C:\Projects -e cs` and `search content-index -d C:\Projects -e cs,sql` create two separate `.cidx` files.

### Collision Handling

`DefaultHasher` is not cryptographic. Hash collisions are possible but extremely unlikely for realistic directory paths. No collision detection is implemented — a collision would silently overwrite the previous index. The probability is ~$1/2^{64}$ per pair of directories.

## Serialization Format

All indexes use [bincode](https://docs.rs/bincode/1/bincode/) v1 for serialization:

```rust
// Write
let encoded: Vec<u8> = bincode::serialize(&index)?;
fs::write(&path, encoded)?;

// Read
let data: Vec<u8> = fs::read(&path)?;
let index: ContentIndex = bincode::deserialize(&data)?;
```

### Bincode Properties

| Property    | Value                                                                                   |
| ----------- | --------------------------------------------------------------------------------------- |
| Format      | Little-endian, variable-length integers                                                 |
| Schema      | Implicit — derived from Rust struct layout                                              |
| Versioning  | None — format changes require reindex                                                   |
| Compression | None — raw serialized bytes                                                             |
| Atomicity   | Whole-file write (`fs::write`) — atomic on most FSes if < 4KB, otherwise not guaranteed |

### Sizes on Disk

Measured on a real codebase (from `search info` and build logs):

| Index Type      | Files Indexed   | Content                 | Disk Size |
| --------------- | --------------- | ----------------------- | --------- |
| FileIndex       | 333,875 entries | Paths + metadata        | 47.8 MB   |
| ContentIndex    | 48,599 files    | 33M tokens, 754K unique | 241.7 MB  |
| DefinitionIndex | 53,799 files    | ~846K definitions + 2.4M call sites | ~324 MB   |

In-memory size is larger than on-disk due to HashMap overhead and struct alignment, but has not been separately measured.

## Data Structures on Disk

### FileIndex

```rust
struct FileIndex {
    root: String,           // Canonical directory path
    created_at: u64,        // Unix timestamp (seconds)
    max_age_secs: u64,      // Staleness threshold
    entries: Vec<FileEntry>, // All files and directories
}

struct FileEntry {
    path: String,           // Full file path
    size: u64,              // File size in bytes
    modified: u64,          // Last modified timestamp
    is_dir: bool,           // Directory flag
}
```

### ContentIndex

```rust
struct ContentIndex {
    root: String,
    created_at: u64,
    max_age_secs: u64,
    files: Vec<String>,                          // file_id → path
    index: HashMap<String, Vec<Posting>>,         // token → postings
    total_tokens: u64,                           // Total tokens indexed
    extensions: Vec<String>,                     // Extensions indexed
    file_token_counts: Vec<u32>,                 // file_id → token count (TF denom)
    forward: Option<HashMap<u32, Vec<String>>>,  // file_id → tokens (watch mode)
    path_to_id: Option<HashMap<PathBuf, u32>>,   // path → file_id (watch mode)
}

struct Posting {
    file_id: u32,           // Index into ContentIndex.files
    lines: Vec<u32>,        // Line numbers where token appears
}
```

**Watch mode fields:** `forward` and `path_to_id` are only populated when the MCP server starts with `--watch`. They are serialized as `None` when saving to disk (not needed for persistent storage, rebuilt on load).

### DefinitionIndex

```rust
struct DefinitionIndex {
    root: String,
    created_at: u64,
    extensions: Vec<String>,
    files: Vec<String>,                               // file_id → path
    definitions: Vec<DefinitionEntry>,                 // All definitions
    name_index: HashMap<String, Vec<u32>>,             // name → def indices
    kind_index: HashMap<DefinitionKind, Vec<u32>>,     // kind → def indices
    attribute_index: HashMap<String, Vec<u32>>,        // attribute → def indices
    base_type_index: HashMap<String, Vec<u32>>,        // base type → def indices
    file_index: HashMap<u32, Vec<u32>>,                // file_id → def indices
    path_to_id: HashMap<PathBuf, u32>,                 // path → file_id
    method_calls: HashMap<u32, Vec<CallSite>>,         // def_idx → call sites (for search_callers "down")
}

struct DefinitionEntry {
    file_id: u32,
    name: String,
    kind: DefinitionKind,
    line_start: u32,
    line_end: u32,
    parent: Option<String>,       // Containing class/struct
    signature: Option<String>,    // Full signature text
    modifiers: Vec<String>,       // public, static, async, etc.
    attributes: Vec<String>,      // C# attributes
    base_types: Vec<String>,      // Implemented interfaces, base class
}

struct CallSite {
    method_name: String,          // Name of the called method
    receiver_type: Option<String>, // Resolved type of receiver (e.g., "IUserService")
    line: u32,                    // Line number of the call site
}
```

## Staleness Model

Each index stores `created_at` and `max_age_secs`. Staleness check:

```rust
fn is_stale(&self) -> bool {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    now - self.created_at > self.max_age_secs
}
```

| Behavior                            | `search fast` / `search grep`                                | `search serve`                   |
| ----------------------------------- | ------------------------------------------------------------ | -------------------------------- |
| Index stale, `--auto-reindex true`  | Rebuild automatically                                        | N/A (index stays in RAM)         |
| Index stale, `--auto-reindex false` | Print warning, use stale                                     | N/A                              |
| Index missing                       | Build automatically (`search fast`) or error (`search grep`) | Build at startup                 |
| With `--watch`                      | N/A                                                          | Incremental updates, never stale |

Default max age: 24 hours (`--max-age-hours 24`).

## Index Discovery

When loading a content index, the system tries two strategies:

### 1. Exact Match

Hash the directory + extensions to get the exact filename:

```rust
fn load_content_index(dir: &str, exts: &str) -> Option<ContentIndex> {
    let path = content_index_path_for(dir, exts);  // Deterministic hash
    let data = fs::read(&path).ok()?;
    bincode::deserialize(&data).ok()
}
```

### 2. Directory Scan (Fallback)

If exact match fails (e.g., user indexed with `cs` but queries without specifying extensions), scan all `.cidx` files and check the `root` field:

```rust
fn find_content_index_for_dir(dir: &str) -> Option<ContentIndex> {
    for entry in fs::read_dir(index_dir()) {
        if path.extension() == "cidx" {
            let index: ContentIndex = bincode::deserialize(&fs::read(&path)?)?;
            if index.root == canonical_dir {
                return Some(index);
            }
        }
    }
    None
}
```

This scan reads and deserializes each `.cidx` file header — slow if many indexes exist. In practice, users have 1-5 indexes.

## Incremental Update Mechanics

### Content Index Update (single file)

```
1. path_to_id[path] → file_id
2. forward[file_id] → old_tokens
3. For each old_token:
     inverted_index[old_token].retain(|p| p.file_id != file_id)
     if posting list empty: inverted_index.remove(old_token)
4. Read new file content from disk
5. Tokenize → new_tokens with line numbers
6. For each new_token:
     inverted_index[new_token].push(Posting{file_id, lines})
7. forward[file_id] = new_tokens.keys()
8. file_token_counts[file_id] = new_total
```

**Time complexity:** O(old_tokens + new_tokens + Σ posting_list_scans). The ~5ms per file figure is from watcher log output during development, not a formal benchmark.

### Definition Index Update (single file)

```
1. path_to_id[path] → file_id
2. file_index[file_id] → old_def_indices
3. Remove old_def_indices from: name_index, kind_index, attribute_index, base_type_index
4. Parse file with tree-sitter → new_definitions
5. Assign new indices, insert into all secondary indexes
```

**Note:** Removed definitions leave "holes" in the `definitions` Vec (indices are not reused). This is acceptable because the Vec is only accessed via the secondary indexes, and the memory overhead of a few hundred empty slots is negligible compared to the total index size.

The `method_calls` entries for removed definitions are also cleaned up during `remove_file_definitions`.

## Disk I/O Patterns

| Operation          | I/O Pattern                                                       | Duration                |
| ------------------ | ----------------------------------------------------------------- | ----------------------- |
| Index build        | Sequential read of all matching files, one large sequential write | 7-16s (measured)        |
| Index load         | One large sequential read + deserialize                           | 0.055-0.689s (measured) |
| Search query       | Pure in-memory (no disk I/O)                                      | 0.5-44ms (measured)     |
| Incremental update | One small random read (file content) + in-memory update           | ~5ms (from logs)        |
| Index save         | One large sequential write (only on full reindex)                 | ~2s (estimated)         |

The MCP server never touches disk during normal query processing. All searches are in-memory.
