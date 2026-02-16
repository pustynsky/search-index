# Performance Benchmarks

All numbers in this document are **measured**, not estimated. Criterion benchmarks use synthetic data for reproducibility; CLI and MCP benchmarks use a real production codebase.

## Test Environments

Benchmarks were measured on two machines to show hardware-dependent variability:

| Parameter   | Machine 1 (primary)                                  | Machine 2 (Azure VM)                                       |
| ----------- | ---------------------------------------------------- | ---------------------------------------------------------- |
| **CPU**     | Intel Core i7-12850HX (16 cores / 24 threads)        | Intel Xeon Platinum 8370C @ 2.80GHz (8 cores / 16 threads) |
| **RAM**     | 128 GB                                               | 64 GB                                                      |
| **Storage** | NVMe SSD                                             | Azure VM — Msft Virtual Disk (NVMe-backed)                 |
| **OS**      | Windows 11                                           | Windows 11 Enterprise                                      |
| **Rust**    | 1.83+ (edition 2024)                                 | same                                                       |
| **Build**   | `--release` with LTO (`opt-level = 3`, `lto = true`) | same                                                       |

Unless noted, numbers are from Machine 1. Cross-machine comparisons are shown where available.

## Codebase Under Test

Real production C# codebase (enterprise backend monorepo):

| Metric                       | Value              |
| ---------------------------- | ------------------ |
| Total files indexed          | 48,599–48,639 (varies by run) |
| File types                   | C# (.cs)           |
| Unique tokens                | 754,350            |
| Total token occurrences      | 33,082,236         |
| Definitions (AST)            | ~846,000           |
| Call sites                   | ~2.4M              |
| Content index size           | 241.7–333.4 MB (varies by trigram inclusion) |
| Definition index size        | ~325 MB            |
| Files parsed for definitions | 48,599–48,649 (varies by run) |

## Content Search: search vs ripgrep

Single-term search for `HttpClient` across the full codebase (1,065 matching files, 11,706 occurrences):

| Tool                                           | Operation                         | Wall Time | Speedup     |
| ---------------------------------------------- | --------------------------------- | --------- | ----------- |
| `rg HttpClient -g '*.cs' -l`                   | Live file scan                    | 27.52s    | baseline    |
| `search find "HttpClient" --contents -e cs -c` | Live parallel walk (24 threads)   | 0.80s     | **34×**     |
| `search grep "HttpClient" -e cs -c`            | Inverted index (total incl. load) | 1.33s     | **21×**     |
| ↳ index load from disk                         | bincode deserialize 241.7 MB      | 0.689s    | —           |
| ↳ search + TF-IDF rank                         | HashMap lookup + scoring          | 0.644ms   | **42,700×** |

> **Note:** In MCP server mode, the index is loaded once at startup. All subsequent queries pay only the search+rank cost (0.6–4ms depending on hardware), not the load cost.

## CLI Search Latency (index pre-loaded from disk)

Measured via `search grep` on 48,599-file C# index (754K unique tokens):

| Query Type                                | Search+Rank Time | Files Matched | Occurrences |
| ----------------------------------------- | ---------------- | ------------- | ----------- |
| Single token (`HttpClient`)               | 0.644ms          | 1,065         | 11,706      |
| Multi-term AND (`HttpClient,ILogger`)     | 0.500ms          | 16            | 226         |
| Multi-term OR (`HttpClient,ILogger,Task`) | 5.628ms          | 13,349        | 151,750     |
| Regex (`i.*cache` → 218 matching tokens)  | 44.24ms          | 1,419         | 4,237       |

**Note:** The "Search+Rank" column is the pure in-memory search time as reported by the tool's internal timers. The total CLI wall time also includes index load from disk (~0.69s).

## MCP Server: search_grep vs ripgrep (11-Test Suite)

Comprehensive comparison of MCP `tools/call` JSON-RPC queries vs `rg` (ripgrep v14.x) on the same codebase. All MCP times are in-memory (index pre-loaded at server startup); rg performs a full filesystem scan per query.

| #   | Test                                                  | MCP files | rg files   | MCP time (ms) | rg time (ms) | Speedup        |
| --- | ----------------------------------------------------- | --------- | ---------- | ------------- | ------------ | -------------- |
| 1   | Token single (`OrderServiceProvider`)                 | 2,714     | 2,741      | **1.76**      | 38,025       | **21,600×**    |
| 2   | Multi-term OR (3 variants)                            | 13        | 26         | **0.03**      | 36,921       | **1,230,700×** |
| 3   | Multi-term AND (`IFeatureResolver` + `MonitoredTask`) | 298       | 0¹         | **1.13**      | 78,717       | **69,700×**    |
| 4   | Substring compound (`FindAsyncWithQuery`)            | 3         | 3          | **1.03**      | 37,561       | **36,500×**    |
| 5   | Substring short (`ProductQuery`)                      | 28        | 28         | **0.94**      | 40,485       | **43,100×**    |
| 6   | Phrase (`new ConcurrentDictionary`)                   | 310       | 310        | **455.26**    | 39,729       | **87×**        |
| 7   | Regex (`I\w*Cache`)                                   | 1,418     | 2,642      | **131.63**    | 37,809       | **287×**       |
| 8   | Full results + context (3 lines, top 5)               | 6 files   | 415 lines  | **6.20**      | 38,590       | **6,200×**     |
| 9   | Exclude Test/Mock filters                             | 3         | 6          | **0.03**      | 27,799       | **926,600×**   |
| 10  | AST definitions + inline body                         | 18 defs   | ~798 lines | **33.20**     | 43,497       | **1,310×**     |
| 11  | Call tree (3 levels deep)                             | 48 nodes  | N/A²       | **0.49**      | N/A          | **∞**          |

> ¹ rg AND returned 0 files due to a PowerShell scripting issue with `ForEach-Object` pipeline, not a real result.
> ² `search_callers` has no rg equivalent — it combines grep index + AST index + recursive traversal in a single 0.49ms operation. Building a 3-level call tree manually with rg would require 7+ sequential queries (estimated 5+ minutes of agent round-trips).

### Test Descriptions

#### Test 1: Token search (single term, common identifier)

- **What it tests**: Basic inverted index lookup, TF-IDF ranking
- **MCP**: `search_grep terms="OrderServiceProvider" countOnly=true`
- **rg**: `rg "OrderServiceProvider" --type cs -l`

#### Test 2: Multi-term OR search (find all variants of a class)

- **What it tests**: Multi-term OR mode, ranking across variants
- **MCP**: `search_grep terms="UserMapperCache,IUserMapperCache,UserMapperCacheEntry" mode="or" countOnly=true`
- **rg**: `rg "UserMapperCache|IUserMapperCache|UserMapperCacheEntry" --type cs -l`

#### Test 3: Multi-term AND search (find files using multiple types together)

- **What it tests**: AND mode intersection
- **MCP**: `search_grep terms="IFeatureResolver,MonitoredTask" mode="and" countOnly=true`
- **rg**: `rg -l "IFeatureResolver" | ForEach-Object { if (rg -q "MonitoredTask" $_) { $_ } }`

#### Test 4: Substring search (compound camelCase identifier)

- **What it tests**: Trigram-based substring matching
- **MCP**: `search_grep terms="FindAsyncWithQuery" substring=true countOnly=true`
  → matched tokens: `findasyncwithqueryactivity`, `findasyncwithqueryactivityname`
- **rg**: `rg "FindAsyncWithQuery" --type cs -l`

#### Test 5: Substring search (short substring inside long identifiers)

- **What it tests**: Trigram matching for 4+ char substrings
- **MCP**: `search_grep terms="ProductQuery" substring=true countOnly=true`
  → matched 46 distinct tokens (productquerybuilder, iproductquerymanager, parsedproductqueryrequest, etc.)
- **rg**: `rg "ProductQuery" --type cs -l`

#### Test 6: Phrase search (exact multi-word sequence)

- **What it tests**: Phrase matching across adjacent tokens (requires line-by-line scan)
- **MCP**: `search_grep terms="new ConcurrentDictionary" phrase=true countOnly=true`
- **rg**: `rg "new ConcurrentDictionary" --type cs -l`

#### Test 7: Regex search (pattern matching)

- **What it tests**: Regex over tokenized index
- **MCP**: `search_grep terms="I.*Cache" regex=true countOnly=true`
- **rg**: `rg "I\w*Cache" --type cs -l`

#### Test 8: Full results with context lines

- **What it tests**: Line-level results, context window, ranking relevance
- **MCP**: `search_grep terms="InitializeIndexAsync" showLines=true contextLines=3 maxResults=5`
- **rg**: `rg "InitializeIndexAsync" --type cs -C 3`

#### Test 9: Exclusion filters (production-only results)

- **What it tests**: Exclude patterns for Test/Mock filtering
- **MCP**: `search_grep terms="StorageIndexManager" exclude=["Test","Mock"] excludeDir=["test"] countOnly=true`
- **rg**: `rg "StorageIndexManager" --type cs -l --glob "!*Test*" --glob "!*Mock*" --glob "!*test*"`

#### Test 10: AST definitions with inline source code

- **What it tests**: Tree-sitter AST index, definition lookup with inline source code
- **MCP**: `search_definitions name="InitializeIndexAsync" kind="method" includeBody=true maxBodyLines=20`
  → Returns 18 structured definitions with signatures, parent classes, line ranges, and source code
- **rg**: `rg "InitializeIndexAsync" --type cs -A 20` (approximate, unstructured)

#### Test 11: Call tree (callers analysis)

- **What it tests**: Recursive caller tracing with depth
- **MCP**: `search_callers method="InitializeIndexAsync" class="StorageIndexManager" depth=3 excludeDir=["test","Test","Mock"]`
  → Returns 48-node hierarchical call tree in 0.49ms
- **rg**: No equivalent. Would require 7+ sequential `rg` + `read_file` calls (estimated 5+ minutes of agent round-trips)

## File Count Differences: MCP vs ripgrep

MCP and ripgrep may return different file counts for the same query. This is expected behavior due to different matching strategies:

| Test       | MCP   | rg    | Reason                                                                                                                 | Fix                                            |
| ---------- | ----- | ----- | ---------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| **Test 1** | 2,714 | 2,741 | MCP matches whole tokens; rg matches raw substrings (catches partial matches in comments/strings)                      | Use `substring=true`                           |
| **Test 2** | 13    | 26    | MCP matches exact tokens only; rg matches substrings (e.g., `UserMapperCache` inside `DeleteUserMapperCacheEntry`) | Use `substring=true` → **26 files, 1.12ms** ✅ |
| **Test 3** | 298   | 0     | rg AND script has PowerShell pipeline issue¹; MCP AND mode works natively with set intersection                        | N/A (MCP is correct)                           |
| **Test 7** | 1,418 | 2,642 | MCP regex runs on tokenized index (whole tokens); rg matches raw substrings anywhere in any context                    | Use `substring=true` instead of regex          |
| **Test 9** | 3     | 6     | MCP exclude filters match more aggressively on path substrings vs rg glob patterns                                     | Check exclude patterns                         |

### Deep Dive: Why does default token search miss files?

MCP tokenizes C# source code into **whole identifiers**. Long compound identifiers become single tokens:

```
DeleteUserMapperCacheEntryName                           → token: "deleteusermappercacheentryname"
PlatformSearchDeleteUserMapperCacheEntryActivity     → token: "platformsearchdeleteusermappercacheentryactivity"
m_userMapperCache                                        → tokens: "m", "usermappercache"
```

When you search for `UserMapperCache` in **default (exact token) mode**, it only matches the token `usermappercache` — not `deleteusermappercacheentryname` (which is a different, longer token).

**Solution**: Use `substring=true` to enable trigram-based substring matching:

```json
// Default mode: 13 files (exact token match only)
{ "terms": "UserMapperCache", "countOnly": true }

// Substring mode: 26 files (matches inside longer tokens) — same as rg!
{ "terms": "UserMapperCache", "substring": true, "countOnly": true }
```

Both modes complete in ~1ms. The substring mode found **28 matched tokens** including:
`deleteusermappercacheentryname`, `platformsearchdeleteusermappercacheentryactivity`,
`m_usermappercache`, `platformsearchusermappercacheinsertforbulkmappings_head_platformsearch_be`, etc.

**Rule of thumb**: When looking for ALL usages of a name (including compound identifiers), use `substring=true`. When looking for exact identifier matches only, use default mode.

## MCP Server: search_definitions and search_callers

Measured via MCP `tools/call` JSON-RPC with index pre-loaded in RAM. No disk I/O on queries.

| # | Task | ripgrep (`rg`) | search-index MCP | Speedup | MCP Tool |
|---|------|---------------|-----------------|---------|----------|
| 1 | Find a method definition by name | 48,993 ms | 38.7 ms | **1,266×** | `search_definitions` |
| 2 | Build a call tree (3 levels deep) | 52,121 ms ¹ | 0.51 ms | **~100,000×** | `search_callers` |
| 3 | Find which method contains line N | 195 ms ² | 7.7 ms | **25×** | `search_definitions` (containsLine) |
| 4 | Find all implementations of an interface | 56,222 ms | 0.63 ms | **~89,000×** | `search_definitions` (baseType) |
| 5 | Find interfaces matching a regex | 45,370 ms | 58.2 ms | **780×** | `search_definitions` (regex) |
| 6 | Find classes with a specific attribute | 38,699 ms | 29.2 ms | **1,325×** | `search_definitions` (attribute) |

> ¹ `rg` only provides flat text search — it cannot build a call tree. The 52s is for a single `rg` query; building a 3-level tree manually would require 3–7 sequential queries totaling 150–350 seconds.
> ² For containsLine, `rg` only reads a single file (not the full repo), so the speedup is smaller.

## Performance Summary by Search Mode

| Mode | Latency (MCP, in-memory) | Speedup vs rg | Notes |
|------|-------------------------|---------------|-------|
| **Token (exact)** | 0.6–1.8 ms | 21,000–42,700× | Single HashMap lookup, O(1) |
| **Multi-term OR** | 0.03–5.6 ms | 6,500–1,230,700× | Depends on term rarity and result set size |
| **Multi-term AND** | 0.5–1.1 ms | 69,700× | Set intersection |
| **Substring (trigram)** | 0.9–1.1 ms | 36,500–43,100× | Trigram index, same ballpark as exact token |
| **Phrase** | ~455 ms | 87× | Weakest mode — requires line-by-line file scan |
| **Regex** | 44–132 ms | 287× | Linear scan of all token keys |
| **Context results** | ~6 ms | 6,200× | Ranked results with context lines |
| **Exclusion filters** | ~0.03 ms | 926,600× | Path-based filtering on indexed data |
| **AST definitions** | 0.6–38.7 ms | 780–89,000× | Depends on query type (name, baseType, regex) |
| **AST defs + includeBody** | ~33 ms | 1,310× | Includes file I/O to read source code |
| **Call tree (3 levels)** | ~0.5 ms | ∞ (no rg equivalent) | Recursive traversal, zero file I/O |

### Unique Capabilities (no rg equivalent)

| Capability               | Tool                 | What it does                                                                                           |
| ------------------------ | -------------------- | ------------------------------------------------------------------------------------------------------ |
| **AST definitions**      | `search_definitions` | Find classes/methods/interfaces by name, kind, parent, base type, attributes — with inline source code |
| **Call trees**           | `search_callers`     | Build hierarchical caller/callee trees across the entire codebase in < 1ms                             |
| **Structured results**   | `search_grep`        | TF-IDF ranked files with occurrence counts, line numbers, context groups                               |
| **Token-level matching** | `search_grep`        | Matches whole C# identifiers (respects camelCase boundaries) vs raw substring matching                 |

### When to Use ripgrep Instead

- Searching **non-indexed file types** (XML, SQL, JSON, YAML, `.csproj`) — unless they are included in `--ext`
- Exact **raw substring** matching needed when `substring=true` behaves differently than expected (MCP tokenizes, so `m_` prefix is a separate token)
- search-index MCP server is not running
- One-off searches where index build time (7–16s) is not justified

## MCP Tool Latency Summary

Verified measurements from two machines:

| Tool | Query Type | Machine 1 (24 threads) | Machine 2 (16 threads) |
|------|-----------|------------------------|------------------------|
| `search_grep` | Single token | 0.6 ms | 4.2 ms |
| `search_grep` | Multi-term OR (3) | 5.6 ms | 11.4 ms |
| `search_grep` | Regex (i.*cache) | 44 ms | 68 ms |
| `search_grep` | Substring (trigram) | ~1 ms | — |
| `search_grep` | Phrase | ~455 ms | — |
| `search_grep` | Exclusion filters | ~0.03 ms | — |
| `search_grep` | Context lines (top 5) | ~6 ms | — |
| `search_definitions` | Find by name | 38.7 ms | — |
| `search_definitions` | Find implementations (baseType) | 0.63 ms | — |
| `search_definitions` | containsLine | 7.7 ms | — |
| `search_definitions` | Attribute filter | 29.2 ms | — |
| `search_definitions` | With includeBody | ~33 ms | — |
| `search_callers` | Call tree (3 levels) | 0.5 ms | — |
| `search_find` | Live filesystem walk | — | 1,037 ms |

## File Name Search

Searching for `notepad` in 333,875 indexed entries (C:\Windows):

| Tool                                     | Operation            | Wall Time |
| ---------------------------------------- | -------------------- | --------- |
| `search fast "notepad" -d C:\Windows -c` | Pre-built file index | 0.091s    |

Index load: 0.055s, search: 0.036s.

## Index Build Times

Three distinct indexes, each built independently:

| Index Type | What it stores | CLI command | MCP tool |
|---|---|---|---|
| **FileIndex** (.idx) | File paths, sizes, timestamps | `search index` | — |
| **ContentIndex** (.cidx) | Inverted token→file map for TF-IDF search | `search content-index` | `search_reindex` |
| **DefinitionIndex** (.didx) | AST definitions + call graph | `search def-index` | `search_reindex_definitions` |

### Build times across machines

| Index Type                 | Files           | Machine 1 (24 threads) | Machine 2 (16 threads) | Disk Size |
| -------------------------- | --------------- | ---------------------- | ---------------------- | --------- |
| FileIndex (C:\Windows)     | 333,875 entries | ~3s                    | —                      | 47.8 MB   |
| ContentIndex (C# files)    | 48,599 files    | 7.0s                   | 15.9s                  | 241.7 MB  |
| DefinitionIndex (C#)       | ~48,600 files   | 16.1s                  | 32.0s                  | ~324 MB   |

**Why is def-index 2× slower than content-index?**

- Content indexing: read file → split tokens (simple string operations)
- Definition indexing: read file → parse full AST with tree-sitter → walk AST tree → extract definitions with modifiers, attributes, base types → extract call sites from method bodies

## Criterion Benchmarks (synthetic, reproducible)

Run with `cargo bench`. Uses synthetic data for cross-machine reproducibility.

### Tokenizer

| Input                              | Time    | Throughput    |
| ---------------------------------- | ------- | ------------- |
| Short line (6 tokens, 36 chars)    | 221 ns  | ~163M chars/s |
| Medium line (15 tokens, 120 chars) | 654 ns  | ~183M chars/s |
| Long line (30+ tokens, 260 chars)  | 1.65 µs | ~157M chars/s |
| 30-line code block                 | 5.40 µs | —             |

### Index Lookup (HashMap::get)

| Operation            | 1K files | 10K files | 50K files |
| -------------------- | -------- | --------- | --------- |
| Single token lookup  | 10.1 ns  | 10.3 ns   | 9.9 ns    |
| Common token lookup  | 9.7 ns   | 12.2 ns   | 10.2 ns   |
| Rare token lookup    | 11.5 ns  | 11.1 ns   | 13.0 ns   |
| Missing token lookup | 10.8 ns  | 11.0 ns   | 10.3 ns   |

**Key insight:** Lookup time is O(1) regardless of index size — consistent ~10ns per lookup.

### TF-IDF Scoring

| Operation                 | 1K files | 10K files | 50K files |
| ------------------------- | -------- | --------- | --------- |
| Score single term         | 2.4 µs   | 26.0 µs   | 297 µs    |
| Score 3 terms (with sort) | 44.3 µs  | 423 µs    | 2.70 ms   |

Scoring time scales linearly with posting list size (number of files containing the term).

### Regex Token Scan

| Pattern                     | 1K files | 10K files | 50K files |
| --------------------------- | -------- | --------- | --------- |
| Broad pattern (`token_4.*`) | 2.9 µs   | 2.9 µs    | 3.1 µs    |
| Exact pattern (`class`)     | 706 ns   | 712 ns    | 776 ns    |

Regex scan time depends on number of unique tokens (500 in synthetic index), not file count.

### Serialization (bincode)

Measured on 5,000-file synthetic index (15.9 MB serialized):

| Operation   | Time    |
| ----------- | ------- |
| Serialize   | 16.3 ms |
| Deserialize | 44.7 ms |

Extrapolated for real 241.7 MB index: ~700ms deserialize (matches measured 689ms load time).

## Index Load Times (measured)

| Index Type             | Files   | Disk Size | Load Time                                            |
| ---------------------- | ------- | --------- | ---------------------------------------------------- |
| ContentIndex           | 48,599  | 241.7 MB  | 0.689s                                               |
| FileIndex (C:\Windows) | 333,875 | 47.8 MB   | 0.055s                                               |
| DefinitionIndex        | ~48,600 | ~324 MB   | ~1.5s (measured on Machine 2)                        |

## Comparison with ripgrep

| Metric                          | ripgrep | search (indexed)       | Speedup     |
| ------------------------------- | ------- | ---------------------- | ----------- |
| First query (cold)              | 27.5s   | 1.33s (incl. load)     | **21×**     |
| Subsequent queries (MCP server) | 27.5s   | 0.6–4.2ms              | **6,500–45,000×** |
| Substring search (MCP)          | 37–40s  | ~1ms                   | **36,500–43,100×** |
| Phrase search (MCP)             | ~40s    | ~455ms                 | **87×**     |
| Regex search (MCP)              | ~38s    | 44–132ms               | **287–860×** |
| AST definitions (MCP)           | 39–56s  | 0.6–38.7ms             | **780–89,000×** |
| Call tree (MCP)                 | N/A     | ~0.5ms                 | ∞           |
| Index build (content, one-time) | N/A     | 7–16s                  | —           |
| Index build (defs, one-time)    | N/A     | 16–32s                 | —           |
| Disk overhead                   | None    | ~566 MB (content+defs) | —           |
| RAM (server mode, estimated)    | None    | ~500 MB (not measured) | —           |

## Bottlenecks and Scaling Limits

| Bottleneck              | Measured Value       | Cause                                | Mitigation                               |
| ----------------------- | -------------------- | ------------------------------------ | ---------------------------------------- |
| Index load              | 689ms for 242 MB     | bincode deserialization + allocation | Memory-map + lazy load (not implemented) |
| Phrase search           | 455ms                | Line-by-line file scan for phrase verification | Consider positional index (not implemented) |
| Regex search            | 44–132ms for 754K tokens | Linear scan of all keys           | FST for prefix queries (not implemented) |
| Multi-term OR (3 terms) | 5.6ms                | Scoring 13K+ posting entries         | Acceptable for interactive use           |
| Content index build     | 7.0s                 | Parallel I/O + tokenization          | Already parallelized (24 threads)        |
| Def index build         | 16.1s                | tree-sitter parsing CPU-bound        | Already parallelized (24 threads)        |

## Cross-Machine Variability

Benchmarks measured on a second machine (16 threads instead of 24) show significantly different numbers due to CPU speed and thread count:

| Metric | i7-12850HX (24 threads) | 2nd machine (16 threads) | Ratio |
|---|---|---|---|
| Single token search | 0.644ms | 4.2ms | 6.5× |
| Multi-term OR (3) | 5.6ms | 11.4ms | 2× |
| Regex (i.*cache) | 44ms | 68ms | 1.5× |
| Content index build | 7.0s | 15.9s | 2.3× |
| Def index build | 16.1s | 32.0s | 2× |
| Index load (startup) | 0.7s | 3.1s (both) | 4.4× |
| Watcher update (1 file) | ~5ms (from logs) | ~0.9s | 180× |

The watcher update discrepancy is notable — the original "~5ms" figure appears to have been the per-file content-only update time, while the new 0.9s measurement includes definition index re-parsing with tree-sitter (which is CPU-intensive). The true per-file update cost depends heavily on file size and CPU speed.

## Reproducibility

All measurements in this document can be reproduced:

```bash
# Build with release optimizations
cargo build --release

# Run criterion benchmarks (synthetic, reproducible)
cargo bench

# Real-codebase benchmarks (requires indexed directory)
search content-index -d <YOUR_DIR> -e cs

# Measure search (PowerShell)
Measure-Command { search grep "HttpClient" -d <YOUR_DIR> -e cs -c }

# Measure ripgrep baseline
Measure-Command { rg "HttpClient" <YOUR_DIR> -g '*.cs' -l }

# Measure index build
Measure-Command { search content-index -d <YOUR_DIR> -e cs }

# MCP benchmarks (start server, then send JSON-RPC)
search serve --dir <YOUR_DIR> --ext cs --watch --definitions
# Paste JSON-RPC messages to stdin and measure response times
