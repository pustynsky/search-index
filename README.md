# Search — High-Performance Code Search Engine

Inverted index + AST-based code intelligence engine for large-scale codebases. Millisecond content search, structural code navigation (classes, methods, call trees), and native MCP server for AI agent integration — in a single statically-linked Rust binary.

**Measured on a real C# codebase with 49,000 files, 846K definitions ([full benchmarks](docs/benchmarks.md)):**

| Metric | Value |
|---|---|
| Indexed content search (MCP, in-memory) | **0.6–4ms** per query (**250–6,500×** faster than live scan/ripgrep) |
| Call tree (3 levels) | **0.5ms** |
| Find interface implementations | **0.6ms** |
| Index build | **7–16s** (content), **16–32s** (AST definitions) — varies by CPU |
| Incremental update | **<1s** per file change (content + AST re-parse) |
| Index load from disk | **0.7–1.6s** (242 MB content index) |
| Binary size | Single static binary, zero runtime dependencies |

> Built on the same [`ignore`](https://crates.io/crates/ignore) crate used by [ripgrep](https://github.com/BurntSushi/ripgrep), with [`tree-sitter`](https://tree-sitter.github.io/) for language-aware code parsing.

## Documentation

| Document | Description |
|---|---|
| [Architecture](docs/architecture.md) | System overview, component design, data flow diagrams |
| [Storage Model](docs/storage.md) | Index formats, serialization, staleness, incremental updates |
| [Concurrency](docs/concurrency.md) | Thread model, lock strategy, watcher design |
| [Trade-offs](docs/tradeoffs.md) | Design decisions with alternatives considered |
| [Benchmarks](docs/benchmarks.md) | Performance data, scaling estimates, industry comparison |
| [E2E Test Plan](docs/e2e-test-plan.md) | 40+ end-to-end test cases (24 CLI + 16 MCP) with automation script |
| [Substring Search Design](docs/substring-search-design.md) | Trigram index design, search algorithm, watcher integration |

## Features

- **Parallel filesystem walk** — uses all available CPU cores for maximum throughput
- **File name index** — pre-built index for instant file lookups (like [Everything](https://www.voidtools.com/))
- **Inverted content index** — maps tokens to files for instant full-text search (like Elasticsearch)
- **TF-IDF ranking** — results sorted by relevance, most relevant files first
- **Regex support** — full Rust regex syntax for pattern matching
- **Respects `.gitignore`** — automatically skips ignored files
- **Extension filtering** — limit search to specific file types
- **MCP Server** — native Model Context Protocol server for AI agents (VS Code Roo, Copilot, Claude)
- **Code definition index** — tree-sitter AST parsing of C# (classes, methods, interfaces, properties, enums) and SQL (stored procedures, tables) for structural code search
- **Parallel parsing** — multi-threaded tree-sitter parsing across all CPU cores (~16-32s for 48K files, varies by CPU)
- **File watcher** — incremental index updates on file changes (<1s per file for content + definition indexes)
- **Substring search** — trigram-indexed substring matching within tokens (e.g., `DatabaseConnection` finds `databaseconnectionfactory`) — ~0.07ms vs ~44ms for regex

## Performance

Measured on a real codebase (48,599 C# files, 754K unique tokens, 33M total). See [full benchmarks](docs/benchmarks.md) for methodology and criterion data.

| Operation | Time | vs ripgrep |
|---|---|---|
| `rg HttpClient -g '*.cs' -l` | 27.52s | baseline |
| `search find --contents -e cs -c` | 0.80s | **34×** |
| `search grep -e cs -c` (total incl. index load) | 1.33s | **21×** |
| ↳ search + TF-IDF rank only | 0.644ms | **42,700×** |

In MCP server mode (index pre-loaded in RAM), all queries pay only the search+rank cost (times vary by hardware — see [full benchmarks](docs/benchmarks.md)):

| Query Type | Machine 1 | Machine 2 |
|---|---|---|
| Single token | 0.6ms | 4.2ms |
| Multi-term OR (3 terms) | 5.6ms | 11.4ms |
| Regex (`i.*cache` → 218 tokens) | 44ms | 68ms |

File name search on `C:\Windows` (333,875 entries):

| Operation | Time |
|---|---|
| `search fast` (pre-built index) | 0.091s |

## Installation

### Prerequisites

- [Rust](https://rustup.rs/) (1.70+)

### Build from source

```bash
git clone <repo-url>
cd search
cargo build --release
```

The binary will be at `target/release/search.exe` (Windows) or `target/release/search` (Linux/Mac).

## Commands

### `search find` — Live Filesystem Search

Walks the filesystem in real-time. No index needed.

```bash
# Search for files by name
search find "config" -d C:\Projects

# Search with extension filter
search find "main" -e rs

# Search file contents
search find "TODO" -d C:\Projects --contents -e cs

# Regex search in file contents
search find "fn\s+\w+" --contents --regex -e rs

# Case-insensitive search
search find "readme" -i -d C:\

# Count matches only
search find ".exe" -d C:\Windows -c

# Limit search depth
search find "node_modules" -d C:\Projects --max-depth 3

# Include hidden and gitignored files
search find "secret" --hidden --no-ignore
```

**Options:**

| Flag                | Description                           |
| ------------------- | ------------------------------------- |
| `-d, --dir <DIR>`   | Root directory (default: `.`)         |
| `-r, --regex`       | Treat pattern as regex                |
| `--contents`        | Search file contents instead of names |
| `--hidden`          | Include hidden files                  |
| `--max-depth <N>`   | Max directory depth (0 = unlimited)   |
| `-t, --threads <N>` | Thread count (0 = auto)               |
| `-i, --ignore-case` | Case-insensitive search               |
| `--no-ignore`       | Include `.gitignore`d files           |
| `-c, --count`       | Show match count only                 |
| `-e, --ext <EXT>`   | Filter by file extension              |

---

### `search index` — Build File Name Index

Pre-builds an index of all file paths for instant lookups.

```bash
# Index a directory
search index -d C:\Projects

# Index with custom max age (hours)
search index -d C:\ --max-age-hours 48

# Include hidden and gitignored files
search index -d C:\Projects --hidden --no-ignore
```

**Options:**

| Flag                  | Description                                          |
| --------------------- | ---------------------------------------------------- |
| `-d, --dir <DIR>`     | Directory to index (default: `.`)                    |
| `--max-age-hours <N>` | Hours before index is considered stale (default: 24) |
| `--hidden`            | Include hidden files                                 |
| `--no-ignore`         | Include `.gitignore`d files                          |
| `-t, --threads <N>`   | Thread count (0 = auto)                              |

---

### `search fast` — Search File Name Index

Searches a pre-built file name index. Instant results.

```bash
# Search by file name (substring match)
search fast "notepad" -d C:\Windows

# With extension filter
search fast "notepad" -d C:\Windows -e exe --files-only

# Regex search
search fast "config\.\w+" -d C:\Projects --regex

# Find large files (> 100MB)
search fast "" -d C:\ --min-size 104857600

# Find directories only
search fast "node_modules" -d C:\Projects --dirs-only

# Count only
search fast ".dll" -d C:\Windows -c
```

If no index exists for the directory, it will be built automatically on first use.

**Options:**

| Flag                 | Description                                    |
| -------------------- | ---------------------------------------------- |
| `-d, --dir <DIR>`    | Directory whose index to search (default: `.`) |
| `-r, --regex`        | Treat pattern as regex                         |
| `-i, --ignore-case`  | Case-insensitive search                        |
| `-c, --count`        | Show match count only                          |
| `-e, --ext <EXT>`    | Filter by extension                            |
| `--auto-reindex`     | Auto-rebuild if stale (default: true)          |
| `--dirs-only`        | Show only directories                          |
| `--files-only`       | Show only files                                |
| `--min-size <BYTES>` | Minimum file size filter                       |
| `--max-size <BYTES>` | Maximum file size filter                       |

---

### `search content-index` — Build Inverted Content Index

Reads file contents, tokenizes them, and builds an inverted index mapping tokens to file locations.

```bash
# Index C# files
search content-index -d C:\Projects -e cs

# Index multiple file types
search content-index -d C:\Projects -e cs,rs,py,js,ts

# Custom token minimum length
search content-index -d C:\Projects -e cs --min-token-len 3

# Include everything
search content-index -d C:\Projects -e cs --hidden --no-ignore
```

**Tokenization rules:**

- Text is split on non-alphanumeric characters (except `_`)
- All tokens are lowercased
- Tokens shorter than `--min-token-len` (default: 2) are discarded
- Example: `private readonly HttpClient _client;` → `["private", "readonly", "httpclient", "_client"]`

**Options:**

| Flag                  | Description                                      |
| --------------------- | ------------------------------------------------ |
| `-d, --dir <DIR>`     | Directory to index (default: `.`)                |
| `-e, --ext <EXTS>`    | File extensions, comma-separated (default: `cs`) |
| `--max-age-hours <N>` | Hours before stale (default: 24)                 |
| `--hidden`            | Include hidden files                             |
| `--no-ignore`         | Include `.gitignore`d files                      |
| `-t, --threads <N>`   | Thread count (0 = auto)                          |
| `--min-token-len <N>` | Minimum token length (default: 2)                |

---

### `search grep` — Search Inverted Content Index

Searches the inverted index for tokens. Results are ranked by TF-IDF score. Supports multi-term search (AND/OR) and regex pattern matching against indexed tokens.

```bash
# Search for a single term (results ranked by relevance)
search grep "HttpClient" -d C:\Projects

# Multi-term OR search (files containing ANY of the terms)
search grep "HttpClient,ILogger,Task" -d C:\Projects -e cs

# Multi-term AND search (files containing ALL terms)
search grep "HttpClient,ILogger" -d C:\Projects -e cs --all

# Regex: find all cache interfaces
search grep "i.*cache" -d C:\Projects -e cs --regex

# Regex: find all factory classes
search grep ".*factory" -d C:\Projects -e cs --regex --max-results 20

# Regex: find all async methods
search grep ".*async" -d C:\Projects -e cs --regex -c

# Show actual matching lines from files
search grep "HttpClient" -d C:\Projects --show-lines

# Top 10 results only
search grep "HttpClient" -d C:\Projects --max-results 10

# Count matches
search grep "HttpClient" -d C:\Projects -c

# Filter by extension
search grep "HttpClient" -d C:\Projects -e cs
```

**Multi-term search:**

- Separate terms with commas: `"term1,term2,term3"`
- **OR mode** (default): file matches if it contains **any** of the terms
- **AND mode** (`--all`): file matches only if it contains **all** terms
- TF-IDF scores are summed across matching terms — files matching more terms rank higher
- Output shows `X/N terms` indicating how many of the search terms were found in each file

**Substring search (`--substring`):**

- Finds tokens that **contain** the search term as a substring
- Uses a trigram index for fast matching (~0.07ms) — much faster than regex scanning (~12–44ms)
- Solves the compound-identifier problem: searching `DatabaseConnection` finds the token `databaseconnectionfactory` even though it's stored as a single token in the inverted index
- For queries shorter than 4 characters, a warning is included in the response (trigram matching is less selective for very short queries)
- Mutually exclusive with `--regex` and `--phrase`
- Example: `search grep "DatabaseConn" -d C:\Projects -e cs --substring`
- In MCP mode: `{ "terms": "DatabaseConn", "substring": true }`
- Response includes `matchedTokens` field listing which index tokens matched the substring

**Regex search (`-r, --regex`):**

- Pattern is matched against all indexed tokens using Rust regex syntax
- Anchored with `^...$` automatically — matches full tokens
- Example: `"i.*cache"` → matches `itenantcache`, `iusercache`, `isessioncache`, etc.
- Multiple regex patterns via commas: `"i.*cache,.*factory"`
- Can combine with `--all` for AND across regex patterns
- Performance: scans 754K tokens in ~12ms, then instant posting lookups

**Options:**

| Flag                | Description                                               |
| ------------------- | --------------------------------------------------------- |
| `-d, --dir <DIR>`   | Directory whose content index to search (default: `.`)    |
| `-c, --count`       | Show match count only                                     |
| `--show-lines`      | Display actual line content from files                    |
| `--auto-reindex`    | Auto-rebuild if stale (default: true)                     |
| `-e, --ext <EXT>`   | Filter results by extension                               |
| `--max-results <N>` | Limit number of results (0 = unlimited)                   |
| `--all`             | AND mode: file must contain ALL terms (default: OR)       |
| `-r, --regex`       | Treat pattern as regex, match against indexed tokens      |
| `--exclude-dir <S>` | Exclude files with this substring in path (repeatable)    |
| `--exclude <S>`     | Exclude files matching this pattern in path (repeatable)  |
| `-C, --context <N>` | Show N context lines around matches (with --show-lines)   |
| `-B, --before <N>`  | Show N lines before each match (with --show-lines)        |
| `-A, --after <N>`   | Show N lines after each match (with --show-lines)         |
| `--phrase`          | Phrase search: find exact phrase via index + verification |
| `--substring`       | Substring search via trigram index (MCP: `substring: true`) |

---

### `search info` — Index Information

Shows all existing indexes with their status.

```bash
search info
```

Example output:

```
Index directory: C:\Users\you\AppData\Local\search-index

  [FILE] C:\Windows — 333875 entries, 47.8 MB, 0.1h ago
  [CONTENT] C:\Projects — 48986 files, 33229888 tokens, exts: [cs, rs], 242.7 MB, 0.5h ago
```

---

### `search def-index` — Build Code Definition Index

Parses C# and SQL files using tree-sitter to extract structural code definitions (classes, methods, interfaces, enums, stored procedures, tables, etc.).

```bash
# Index C# files
search def-index --dir C:\Projects --ext cs

# Index C# + SQL files
search def-index --dir C:\Projects --ext cs,sql

# Custom thread count
search def-index --dir C:\Projects --ext cs --threads 8
```

**What it extracts:**

| Language | Definition Types |
| -------- | ---|
| C# (.cs) | classes, interfaces, structs, enums, records, methods, constructors, properties, fields, delegates, events, enum members |
| SQL (.sql) | stored procedures, tables, views, functions, user-defined types (requires compatible tree-sitter grammar) |

Each definition includes: name, kind, file path, line range, full signature, modifiers (public/static/async/etc.), attributes (`[ServiceProvider]`, `[ApiController]`, etc.), base types/interfaces, and parent class.

**Performance:**

| Metric | Value |
| --- | --- |
| ~48,600 files | ~16-32s (varies by CPU/threads) |
| Definitions extracted | ~846,000 |
| Call sites extracted | ~2.4M |
| Index size | ~324 MB |

**Options:**

| Flag              | Description                                   |
| ----------------- | --------------------------------------------- |
| `-d, --dir <DIR>` | Directory to scan recursively (default: `.`)   |
| `-e, --ext <EXTS>`| Extensions to parse (default: `cs,sql`)        |
| `-t, --threads <N>`| Parallel parsing threads, 0 = auto (default: 0)|

---

### `search serve` — MCP Server (AI Agent Integration)

Starts a Model Context Protocol (MCP) server over stdio. Loads the content index into memory for instant queries (~0.001s per search). AI agents connect via JSON-RPC 2.0.

```bash
# Start MCP server for C# files
search serve --dir C:\Projects --ext cs

# With file watching (auto-updates index when files change)
search serve --dir C:\Projects --ext cs --watch

# With code definition index (enables search_definitions tool)
search serve --dir C:\Projects --ext cs --watch --definitions

# Multiple extensions
search serve --dir C:\Projects --ext cs,sql,csproj --watch

# Custom debounce and bulk threshold
search serve --dir C:\Projects --ext cs --watch --debounce-ms 300 --bulk-threshold 200

# With performance metrics in every response (responseBytes, estimatedTokens)
search serve --dir C:\Projects --ext cs --watch --definitions --metrics
```

**Options:**

| Flag                   | Description                                                      |
| ---------------------- | ---------------------------------------------------------------- |
| `-d, --dir <DIR>`      | Directory to index and serve (default: `.`)                      |
| `-e, --ext <EXTS>`     | File extensions, comma-separated (default: `cs`)                 |
| `--watch`              | Watch for file changes and update indexes incrementally          |
| `--definitions`        | Load (or build on first use) code definition index (tree-sitter AST). Cached to disk, instant on subsequent starts. |
| `--metrics`            | Add `responseBytes` and `estimatedTokens` to every tool response summary. Off by default to keep responses lean for LLMs. |
| `--debounce-ms <MS>`   | Debounce delay for file watcher (default: 500)                   |
| `--bulk-threshold <N>` | File changes triggering full reindex (default: 100)              |
| `--log-level <LEVEL>`  | Log level: error, warn, info, debug (default: info)              |

**Exposed MCP Tools:**

| Tool                  | Description                                                      |
| --------------------- | ---------------------------------------------------------------- |
| `search_grep`         | Search content index with TF-IDF ranking, regex, phrase, AND/OR  |
| `search_definitions`  | Search code definitions: classes, methods, interfaces, enums, SPs. Supports `containsLine` to find which method/class contains a given line number. Supports `includeBody` to return source code inline. (requires `--definitions`) |
| `search_callers`      | Find all callers of a method and build a recursive call tree (up or down). Combines grep index + AST definition index to trace call chains in a single request. (requires `--definitions`) |
| `search_find`         | Live filesystem walk (⚠️ slow for large dirs)                    |
| `search_fast`         | Search pre-built file name index (instant)                       |
| `search_info`         | Show all indexes with status, sizes, age                         |
| `search_reindex`      | Force rebuild + reload content index                             |
| `search_reindex_definitions` | Force rebuild + reload definition index (requires `--definitions`) |

**Setup in VS Code (step-by-step):**

1. **Install search** (if not already):

   ```bash
   cargo install --path .
   # Or copy search.exe to a folder in your PATH
   ```

2. **Build a content index** for your project:

   ```bash
   search content-index -d C:\Projects\MyApp -e cs,sql,csproj
   ```

3. **Create `.vscode/mcp.json`** in your workspace root:

   ```json
   {
     "servers": {
       "search-index": {
         "command": "C:\\Users\\you\\.cargo\\bin\\search.exe",
         "args": [
           "serve",
           "--dir",
           "C:\\Projects\\MyApp",
           "--ext",
           "cs",
           "--watch"
         ]
       }
     }
   }
   ```

4. **Restart VS Code** — the MCP server starts automatically. Your AI agent (Copilot, Roo, Claude) now has access to all 8 MCP tools: `search_grep`, `search_definitions`, `search_callers`, `search_find`, `search_fast`, `search_info`, `search_reindex`, and `search_reindex_definitions`.

5. **Verify** — ask the AI: _"Use search_grep to find all files containing HttpClient"_

**What the AI agent sees:**

When the AI connects, it discovers 8 tools with full JSON schemas. Each tool has a detailed description explaining what it does, required/optional parameters, and examples. The AI can then call these tools with structured JSON to search your codebase.

Example interaction:

```
AI:  "Let me search for HttpClient in your codebase..."
     → calls search_grep { terms: "HttpClient", maxResults: 10 }
     ← receives JSON with file paths, scores, line numbers
AI:  "Found 1,082 files. The most relevant is CustomHttpClient.cs (score: 0.49)..."
```

**`search_callers` -- find call tree (requires `--definitions`):**

Traces who calls a method (or what a method calls) and builds a hierarchical call tree. Combines the content index (grep) with the definition index (AST) to determine which method/class contains each call site. Replaces 7+ sequential `search_grep` + `read_file` calls with a single request.

```json
// Find all callers of ExecuteQueryAsync, 5 levels deep, excluding tests
{
  "method": "ExecuteQueryAsync",
  "direction": "up",
  "depth": 5,
  "excludeDir": ["\\test\\", "\\Mock\\"]
}

// Result: hierarchical call tree
{
  "callTree": [
    {
      "method": "RunQueryAsync",
      "class": "QueryService",
      "file": "QueryService.cs",
      "line": 386,
      "callers": [
        {
          "method": "HandleRequestAsync",
          "class": "QueryController",
          "line": 154,
          "callers": [
            { "method": "ProcessBatchAsync", "class": "BatchProcessor", "line": 275 }
          ]
        }
      ]
    },
    { "method": "ExecuteQueryAsync", "class": "QueryProxy", "file": "QueryProxy.cs", "line": 74 }
  ],
  "summary": { "totalNodes": 19, "searchTimeMs": 0.13, "truncated": false }
}
```

| Parameter              | Description                                                    |
| ---------------------- | -------------------------------------------------------------- |
| `method` (required)    | Method name to trace                                           |
| `class`                | Scope to a specific class. Without it, results may mix methods from different classes with the same name. DI-aware: `class: "UserService"` also finds callers using `IUserService`. Works for both `"up"` and `"down"` directions. |
| `direction`            | `"up"` = find callers (default), `"down"` = find callees      |
| `depth`                | Max recursion depth (default: 3, max: 10)                      |
| `maxCallersPerLevel`   | Max callers per node (default: 10). Prevents explosion.        |
| `maxTotalNodes`        | Max total nodes in tree (default: 200). Caps output size.      |
| `excludeDir`           | Directory substrings to exclude, e.g. `["\\test\\", "\\Mock\\"]` |
| `excludeFile`          | File path substrings to exclude                                |
| `resolveInterfaces`    | Auto-resolve interface -> implementation (default: true)       |
| `ext`                  | File extension filter (default: server's `--ext`)              |

**`search_definitions` -- search code definitions (requires `--definitions`):**

| Parameter            | Type    | Default | Description                                                        |
| -------------------- | ------- | ------- | ------------------------------------------------------------------ |
| `name`               | string  | —       | Substring or comma-separated OR search                             |
| `kind`               | string  | —       | Filter by definition kind (class, method, property, etc.)          |
| `attribute`          | string  | —       | Filter by C# attribute                                             |
| `baseType`           | string  | —       | Filter by base type/interface                                      |
| `file`               | string  | —       | Filter by file path substring                                      |
| `parent`             | string  | —       | Filter by parent class name                                        |
| `containsLine`       | integer | —       | Find definition containing a line number (requires `file`)         |
| `regex`              | boolean | false   | Treat `name` as regex                                              |
| `maxResults`         | integer | 100     | Max results returned                                               |
| `excludeDir`         | array   | —       | Exclude directories                                                |
| `includeBody`        | boolean | false   | Include source code body inline                                    |
| `maxBodyLines`       | integer | 100     | Max lines per definition body (0 = unlimited)                      |
| `maxTotalBodyLines`  | integer | 500     | Max total body lines across all results (0 = unlimited)            |

**`containsLine` -- find containing method by line number:**

Find which method/class contains a given line number. No more `read_file` just to figure out "what method is on line 812".

```json
// Request
{ "file": "QueryService.cs", "containsLine": 812 }

// Response: definitions containing that line, sorted by specificity (innermost first)
{
  "containingDefinitions": [
    { "name": "ExecuteQueryAsync", "kind": "method", "lines": "766-830", "parent": "QueryService" },
    { "name": "QueryService", "kind": "class", "lines": "1-900" }
  ]
}
```

**`includeBody` -- return source code inline:**

Retrieve the actual source code of definitions without a separate `read_file` call. Three-level protection prevents response explosion:
- **`maxBodyLines`** — caps lines per individual definition (default: 100, 0 = unlimited)
- **`maxTotalBodyLines`** — caps total body lines across all results (default: 500, 0 = unlimited)
- **`maxResults`** — caps the number of definitions returned (default: 100)

When a definition's body exceeds `maxBodyLines`, the `body` array is truncated and `bodyTruncated: true` is set. When the global `maxTotalBodyLines` budget is exhausted, remaining definitions receive `bodyOmitted: true` with a `bodyWarning` message. If the source file cannot be read, `bodyError` is returned instead.

```json
// Request
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "search_definitions",
    "arguments": {
      "name": "GetCatalogEntriesAsync",
      "includeBody": true,
      "maxBodyLines": 10
    }
  }
}

// Response
{
  "definitions": [
    {
      "name": "GetCatalogEntriesAsync",
      "kind": "method",
      "file": "CatalogService.cs",
      "lines": "142-189",
      "parent": "CatalogService",
      "bodyStartLine": 142,
      "body": [
        "public async Task<List<CatalogEntry>> GetCatalogEntriesAsync(int tenantId)",
        "{",
        "    var entries = await _repository.GetEntriesAsync(tenantId);",
        "    if (entries == null)",
        "    {",
        "        _logger.LogWarning(\"No entries found for tenant {TenantId}\", tenantId);",
        "        return new List<CatalogEntry>();",
        "    }",
        "    return entries.Where(e => e.IsActive).ToList();",
        "}"
      ],
      "bodyTruncated": false
    }
  ],
  "summary": {
    "total": 1,
    "searchTimeMs": 0.4,
    "totalBodyLines": 10,
    "totalBodyLinesReturned": 10
  }
}
```

**Manual testing (without AI):**

```bash
search serve --dir . --ext rs --definitions
# Then paste JSON-RPC messages to stdin:
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"tokenize"}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"ExecuteQueryAsync","depth":3}}}
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_definitions","arguments":{"file":"QueryService.cs","containsLine":812}}}
{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"search_definitions","arguments":{"name":"GetCatalogEntriesAsync","includeBody":true,"maxBodyLines":10}}}
```

---

## Architecture

### Two Independent Index Types

```
%LOCALAPPDATA%\search-index\
├── xxxxxxxxxxxxxxxx.idx      ← File name index (FileIndex)
└── xxxxxxxxxxxxxxxx.cidx     ← Inverted content index (ContentIndex)
```

|                  | File Index (`.idx`)           | Content Index (`.cidx`)          |
| ---------------- | ----------------------------- | -------------------------------- |
| Created by       | `search index`                | `search content-index`           |
| Searched by      | `search fast`                 | `search grep`                    |
| Stores           | File paths, sizes, timestamps | Token → (file, line numbers) map |
| Use case         | Find files by name            | Find files by content            |
| Extension filter | No (indexes all files)        | Yes (`-e cs,rs,py`)              |
| Independent      | ✅                            | ✅                               |

### How the Inverted Index Works

**Forward index** (what's in each file):

```
file_1.cs → ["using", "system", "class", "httpclient", "getasync"]
file_2.cs → ["namespace", "test", "httpclient", "postasync"]
```

Finding `httpclient` requires scanning **ALL** files — O(n).

**Inverted index** (where each token appears):

```
"httpclient" → [file_1.cs (lines: 5, 12), file_2.cs (lines: 3)]
"getasync"   → [file_1.cs (lines: 15)]
```

Finding `httpclient` is a **single HashMap lookup** — O(1).

### TF-IDF Ranking

Results are scored using Term Frequency–Inverse Document Frequency:

```
score(term, file) = TF(term, file) × IDF(term)

TF  = occurrences_in_file / total_tokens_in_file
IDF = ln(total_files / files_containing_term)
```

This means:

- A small file where `HttpClient` is 50% of the content scores **higher** than a large file where it's 1%
- A rare term scores higher than a common one
- Results are sorted by score descending — most relevant files first

### Staleness and Auto-Reindex

- Each index has a configurable max age (default: 24 hours)
- When searching with `search fast` or `search grep`, if the index is older than max age:
  - With `--auto-reindex true` (default): automatically rebuilds
  - Without: shows a warning
- Use `search info` to check index ages
- Force rebuild anytime: `search index -d <dir>` or `search content-index -d <dir> -e <exts>`

## Dependencies

| Crate | Purpose |
|---|---|
| [ignore](https://crates.io/crates/ignore) | Parallel directory walking (from ripgrep) |
| [clap](https://crates.io/crates/clap) | CLI argument parsing |
| [regex](https://crates.io/crates/regex) | Regular expression support |
| [serde](https://crates.io/crates/serde) + [bincode](https://crates.io/crates/bincode) | Fast binary serialization for indexes |
| [serde_json](https://crates.io/crates/serde_json) | JSON serialization for MCP protocol |
| [notify](https://crates.io/crates/notify) | Cross-platform filesystem notifications |
| [dirs](https://crates.io/crates/dirs) | Platform-specific data directory paths |
| [tree-sitter](https://crates.io/crates/tree-sitter) | Incremental parsing for code definition extraction |
| [tracing](https://crates.io/crates/tracing) | Structured diagnostic logging |
| [criterion](https://crates.io/crates/criterion) | Statistical benchmarking (dev) |
| [proptest](https://crates.io/crates/proptest) | Property-based testing (dev) |

## Testing

```bash
# Run all tests (200 tests: 167 main + 32 lib + 1 doctest)
cargo test

# Run property-based tests only
cargo test property_tests

# Run benchmarks
cargo bench

# Run specific test
cargo test test_tokenize_code

# Run tests with output
cargo test -- --nocapture
```

### Test Coverage

| Category | Tests |
|---|---|
| Unit tests | `clean_path`, `tokenize`, staleness, serialization roundtrips, TF-IDF ranking |
| Integration | Build + search ContentIndex, build FileIndex, MCP server end-to-end |
| MCP Protocol | JSON-RPC parsing, initialize, tools/list, tools/call, notifications, errors |
| MCP Handlers | Tool definitions, grep dispatch, callers, definitions, containsLine |
| Substring/Trigram | Trigram generation, trigram index build, sorted intersection, substring search (partial match, full match, no match, short query, case-insensitive, multi-term, mutually exclusive modes, dirty rebuild), 13 e2e integration tests |
| File Watcher | Forward index, incremental update, file removal, bulk threshold, trigram dirty flag |
| Definitions | C# parsing, SQL parsing, incremental update, serialization |
| Property tests (proptest) | Tokenizer invariants (always lowercase, min length, deterministic, valid chars, monotonic), posting roundtrip, index consistency, TF-IDF ordering, clean_path idempotency |
| Benchmarks (criterion) | Tokenizer throughput, index lookup latency, TF-IDF scoring, regex scan, serialization |

## License

MIT
