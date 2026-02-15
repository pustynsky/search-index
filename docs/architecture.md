# Architecture

> High-performance code search engine with inverted indexing, AST-based definition extraction, and an MCP server for AI agent integration.

## System Overview

```mermaid
graph TB
    subgraph CLI["CLI Layer (clap)"]
        FIND[search find]
        INDEX[search index]
        FAST[search fast]
        CIDX[search content-index]
        GREP[search grep]
        DIDX[search def-index]
        SERVE[search serve]
        INFO[search info]
    end

    subgraph Core["Core Engine"]
        WALK[Parallel Walker<br/>ignore crate]
        TOK[Tokenizer]
        TFIDF[TF-IDF Scorer]
        TSP[tree-sitter Parser]
    end

    subgraph Indexes["Index Layer"]
        FI[FileIndex<br/>.idx]
        CI[ContentIndex<br/>.cidx]
        DI[DefinitionIndex<br/>.didx]
    end

    subgraph MCP["MCP Server Layer"]
        PROTO[JSON-RPC 2.0<br/>Protocol]
        HAND[Tool Handlers]
        WATCH[File Watcher<br/>notify crate]
    end

    subgraph Storage["Persistent Storage"]
        DISK["%LOCALAPPDATA%/search-index/"]
    end

    FIND --> WALK
    INDEX --> WALK --> FI
    CIDX --> WALK --> TOK --> CI
    DIDX --> WALK --> TSP --> DI
    FAST --> FI
    GREP --> CI --> TFIDF
    SERVE --> PROTO --> HAND
    HAND --> CI
    HAND --> DI
    WATCH --> CI
    WATCH --> DI
    FI --> DISK
    CI --> DISK
    DI --> DISK
```

## Component Architecture

### 1. Index Layer

Three independent index types, each optimized for a different query pattern:

| Index             | File    | Data Structure                  | Lookup                  | Purpose                  |
| ----------------- | ------- | ------------------------------- | ----------------------- | ------------------------ |
| `FileIndex`       | `.idx`  | `Vec<FileEntry>`                | O(n) scan               | File name search         |
| `ContentIndex`    | `.cidx` | `HashMap<String, Vec<Posting>>` | O(1) per token          | Full-text content search |
| `DefinitionIndex` | `.didx` | Multi-index `HashMap` set       | O(1) per name/kind/attr | Structural code search   |

All indexes are:

- **Serialized with bincode** — fast binary format, zero-copy deserialization
- **Stored deterministically** — file path is `hash(canonical_dir [+ extensions])` as hex
- **Self-describing** — each index embeds its root directory, creation timestamp, and staleness threshold
- **Independent** — can be built, loaded, or deleted without affecting other indexes

### 2. Content Index (Inverted Index)

The core data structure. Maps every token to the files and line numbers where it appears.

```
Forward view (conceptual):
  file_0.cs → [using, system, class, httpclient, getasync]
  file_1.cs → [namespace, test, httpclient, postasync]

Inverted view (actual storage):
  "httpclient" → [Posting{file_id:0, lines:[5,12]}, Posting{file_id:1, lines:[3]}]
  "getasync"   → [Posting{file_id:0, lines:[15]}]
```

**Key properties:**

- Token lookup is a single `HashMap::get()` — O(1)
- Each `Posting` stores both `file_id` and `lines` — enables line-level results without file I/O
- File paths stored in a separate `Vec<String>` indexed by `file_id` — deduplication
- `file_token_counts[file_id]` stores per-file token count for TF normalization

**Optional watch-mode fields:**

- `forward: HashMap<u32, Vec<String>>` — reverse mapping for incremental removal
- `path_to_id: HashMap<PathBuf, u32>` — path-based file lookup for watcher events

### 3. Definition Index (AST Index)

Structural code search using tree-sitter AST parsing. Six cross-referencing indexes over the same `Vec<DefinitionEntry>`, plus a pre-computed call graph:

```mermaid
graph LR
    subgraph DefinitionIndex
        DEFS["definitions: Vec&lt;DefinitionEntry&gt;"]
        NI["name_index: HashMap&lt;String, Vec&lt;u32&gt;&gt;"]
        KI["kind_index: HashMap&lt;Kind, Vec&lt;u32&gt;&gt;"]
        AI["attribute_index: HashMap&lt;String, Vec&lt;u32&gt;&gt;"]
        BTI["base_type_index: HashMap&lt;String, Vec&lt;u32&gt;&gt;"]
        FII["file_index: HashMap&lt;u32, Vec&lt;u32&gt;&gt;"]
        MC["method_calls: HashMap&lt;u32, Vec&lt;CallSite&gt;&gt;"]
    end

    NI -->|"index into"| DEFS
    KI -->|"index into"| DEFS
    AI -->|"index into"| DEFS
    BTI -->|"index into"| DEFS
    FII -->|"index into"| DEFS
    MC -->|"def_idx →"| DEFS
```

Each `DefinitionEntry` contains: `name`, `kind`, `file_id`, `line_start..line_end`, `parent` (containing class), `signature`, `modifiers`, `attributes`, `base_types`.

Each `CallSite` contains: `method_name`, `receiver_type` (resolved via field/constructor type declarations, DI-aware), `line`.

The `method_calls` map stores pre-computed call sites for each method/constructor, extracted during `def-index` build by walking AST `invocation_expression` and `member_access_expression` nodes. This enables instant callee lookups (direction "down") without runtime file I/O.

The multi-index design enables compound queries: "find all public async methods in classes that implement `IQueryHandler` and have `[ServiceProvider]` attribute" — resolved via set intersection of index lookups.

### 4. MCP Server

JSON-RPC 2.0 event loop over stdio. Designed for AI agent integration (VS Code Copilot, Roo, Claude).

```mermaid
sequenceDiagram
    participant Agent as AI Agent
    participant Server as MCP Server
    participant Index as In-Memory Index
    participant Watcher as File Watcher
    participant FS as Filesystem

    Agent->>Server: initialize
    Server->>Agent: capabilities + tools
    Agent->>Server: tools/list
    Server->>Agent: 7 tool definitions

    Agent->>Server: tools/call search_grep
    Server->>Index: HashMap lookup + TF-IDF (~0.6ms measured)
    Index->>Server: Postings + TF-IDF scores
    Server->>Agent: JSON results

    FS->>Watcher: file changed event
    Watcher->>Index: incremental update (~5ms from logs)
```

**Design decisions:**

- **Stdio transport** — no HTTP overhead, direct pipe from VS Code process manager
- **Single-threaded event loop** — JSON-RPC is sequential; index reads use `RwLock` for watcher concurrency
- **Indexes held in `Arc<RwLock<T>>`** — watcher thread writes, server thread reads
- **All logging to stderr** — stdout is exclusively for JSON-RPC protocol messages

### 5. File Watcher

OS-level filesystem notifications (via `notify` crate / `ReadDirectoryChangesW` on Windows) with debounced batch processing.

```mermaid
stateDiagram-v2
    [*] --> Watching
    Watching --> Collecting: file event
    Collecting --> Collecting: more events (within debounce window)
    Collecting --> Processing: debounce timeout
    Processing --> IncrementalUpdate: changes ≤ threshold
    Processing --> FullReindex: changes > threshold
    IncrementalUpdate --> Watching
    FullReindex --> Watching
```

**Incremental update path** (per file, ~5ms):

1. Read file content from disk
2. Remove old tokens from inverted index (via forward index)
3. Re-tokenize file
4. Add new tokens to inverted index
5. Update forward index
6. If definition index is loaded: re-parse with tree-sitter, update definition entries

**Bulk reindex path** (when changes > `bulk_threshold`, default 100):

- Full rebuild of content index from scratch
- Triggered by git checkout, branch switch, large merges

## Data Flow

### Index Build Pipeline

```mermaid
graph LR
    A[Directory] -->|parallel walk| B[File Paths]
    B -->|filter by extension| C[Matching Files]
    C -->|read content| D[File Contents]
    D -->|tokenize| E[Token Streams]
    E -->|build inverted index| F[ContentIndex]
    F -->|bincode serialize| G[.cidx file]

    C -->|tree-sitter parse| H[AST Trees]
    H -->|extract definitions| I[DefinitionEntries]
    I -->|build multi-index| J[DefinitionIndex]
    J -->|bincode serialize| K[.didx file]
```

### Query Pipeline

```mermaid
graph LR
    A[Search Terms] -->|split + lowercase| B[Tokens]
    B -->|HashMap lookup| C[Postings per token]
    C -->|merge by mode| D[File candidates]
    D -->|TF-IDF score| E[Ranked results]
    E -->|apply filters| F[Filtered results]
    F -->|optional: read lines| G[Results with context]
```

**TF-IDF scoring:**

```
score(term, file) = TF(term, file) × IDF(term)

TF  = occurrences_in_file / total_tokens_in_file
IDF = ln(total_files / files_containing_term)
```

Multi-term: scores are summed across matching terms. Files matching more terms rank higher naturally.

### Call Tree Pipeline (search_callers)

**Direction "up" (find callers):**
```mermaid
graph TB
    A[Method Name] -->|grep index lookup| B[Files containing token]
    B -->|for each file/line| C[Definition index lookup]
    C -->|find_containing_method| D[Which method contains this call site?]
    D -->|recurse| A
    D -->|collect| E[Hierarchical call tree]
```

**Direction "down" (find callees):**
```mermaid
graph TB
    A[Method Name] -->|definition index| B[Find method definition]
    B -->|method_calls lookup| C[Pre-computed CallSites]
    C -->|resolve_call_site| D[Match to method definitions]
    D -->|recurse| A
    D -->|collect| E[Hierarchical callee tree]
```

Direction "up" combines the content index (where does this token appear?) with the definition index (which method spans this line range?). Supports `class` parameter for disambiguation and `resolveInterfaces` for DI-aware tracing.

Direction "down" uses the pre-computed call graph — zero runtime file I/O. Call sites are extracted during `def-index` build with field type resolution (DI constructor parameter types → field types → receiver types).

## Module Structure

```
src/
├── lib.rs               # Public types: FileEntry, FileIndex, ContentIndex, Posting
│                          tokenize(), clean_path() — shared by binary and benchmarks
├── main.rs              # CLI args, search commands (find, fast, grep, info, serve)
│                          fn main() dispatches to cmd_* functions returning Result<SearchError>
├── index.rs             # Index storage: save/load/build for FileIndex and ContentIndex
│                          index_dir(), *_path_for(), build_index(), build_content_index()
├── error.rs             # SearchError enum (thiserror) — unified error type
├── definitions.rs       # DefinitionIndex, tree-sitter parsing (C# + SQL)
│                          AST walking, definition extraction, incremental updates
└── mcp/
    ├── mod.rs            # Module exports
    ├── protocol.rs       # JSON-RPC 2.0 types (request, response, error)
    ├── server.rs         # Stdio event loop, method dispatch, graceful shutdown on write errors
    ├── handlers.rs       # Tool implementations (grep, find, fast, callers, defs)
    └── watcher.rs        # File watcher, incremental index updates
```

**Dependency direction:** `main.rs` → `index.rs` → `lib.rs` (types). `mcp/*` → `index.rs` + `definitions.rs`. No circular dependencies. MCP layer depends on core index types but core has no knowledge of MCP.

## Supported Languages

| Language   | Parser                  | Definition Types                                                                                           | Status |
| ---------- | ----------------------- | ---------------------------------------------------------------------------------------------------------- | ------ |
| C# (.cs)   | tree-sitter-c-sharp     | class, interface, struct, enum, record, method, constructor, property, field, delegate, event, enum member | ✅ Active |
| SQL (.sql) | *(disabled)*            | stored procedure, table, view, function, user-defined type, column, index                                  | ⏸️ Disabled — `tree-sitter-sequel-tsql` 0.4 requires language version 15, incompatible with tree-sitter 0.24 (supports 13-14). Parsing code is retained for future use. |

Content indexing (tokenizer) is language-agnostic — works with any text file.
