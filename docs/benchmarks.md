# Performance Benchmarks

All numbers in this document are **measured**, not estimated. Criterion benchmarks use synthetic data for reproducibility; CLI and MCP benchmarks use a real production codebase.

> **Last measured: 2026-03.** Refactors landed since then (April 2026: complexity reduction in `apply_text_edits` / `handle_xray_fast` / `handle_xray_reindex_inner` / `cmd_serve`, periodic watcher rescan, Rust parser via `lang-rust`; May 2026: lock-order RAII enforcement in `mcp::lock_order`, on-demand XML parser via `lang-xml`, doc-audit pass) have **not** been re-benchmarked against the production C# corpus. Treat the absolute numbers as a March baseline; relative comparisons (xray vs ripgrep, index vs live walk) remain representative.

## D20 C# Semantic Scale Baseline

The D20 scale harness generates two deterministic C# corpora from seed `0xD20C5`: 100,000 methods with overload-group sizes 1/2/4/8/16 and 100,000 methods spread across namespace-colliding types. The compact expected manifest is stored in `benches/fixtures/d20-corpus-manifest.json`; generated source and machine-specific results remain under `target/`.

Quick development check:

```powershell
pwsh scripts/bench-d20-csharp-semantics.ps1 `
  -BenchmarkProfile Development `
  -MethodsPerCorpus 1000 `
  -BuildRelease `
  -ColdBuildCount 1 `
  -WarmQueryCount 5
```

Formal baseline recording:

```powershell
pwsh scripts/bench-d20-csharp-semantics.ps1 `
  -BenchmarkProfile Release `
  -BuildRelease `
  -ResultPath target/d20-scale-result.json
```

The release profile performs seven independent cold definition-index builds per corpus, one unmeasured MCP warm-up per fixed query, 100 measured warm queries, and 20 observable watcher transitions in each direction for an overload add/remove probe. The JSON result includes raw build/query/incremental samples, persisted index bytes, CLI/server RSS, candidate counts, corpus and binary hashes, toolchain/machine metadata, source-restoration evidence, and cleanup status.

A standalone run has `budgetEvaluation.status = "baselineRecorded"`. It is not evidence that the D20 performance budgets pass. Comparative claims require matching corpus hash, `Cargo.lock`, Rust toolchain, release profile, hardware, and Defender/indexing policy.

## Test environments

Benchmarks were measured on two machines to show hardware-dependent variability:

| Parameter   | Machine 1 (primary)                                  | Machine 2 (Azure VM)                                       |
| ----------- | ---------------------------------------------------- | ---------------------------------------------------------- |
| **CPU**     | Intel Core i7-12850HX (16 cores / 24 threads)        | Intel Xeon Platinum 8370C @ 2.80GHz (8 cores / 16 threads) |
| **RAM**     | 128 GB                                               | 64 GB                                                      |
| **Storage** | NVMe SSD                                             | DevDrive (ReFS) on Azure VM NVMe-backed disk               |
| **OS**      | Windows 11                                           | Windows 11 Enterprise                                      |
| **Rust**    | 1.91+ (edition 2024)                                 | same                                                       |
| **Build**   | `--release` with LTO (`opt-level = 3`, `lto = true`) | same                                                       |

Unless noted, numbers are from Machine 1. Cross-machine comparisons are shown where available.

## Codebase under test

Real production C# codebase (enterprise backend monorepo):

| Metric                       | Value                                                   |
| ---------------------------- | ------------------------------------------------------- |
| Total files indexed          | 48,599–48,639 (varies by run)                           |
| File types                   | C# (.cs)                                                |
| Unique tokens                | 754,350                                                 |
| Total token occurrences      | 33,082,236                                              |
| Definitions (AST)            | ~846,000                                                |
| Call sites                   | ~2.4M                                                   |
| Content index size           | 223.7 MB on disk (LZ4 compressed; ~350 MB uncompressed) |
| Definition index size        | 103.1 MB on disk (LZ4 compressed; ~328 MB uncompressed) |
| Files parsed for definitions | 48,599–48,649 (varies by run)                           |

## Content Search: xray vs ripgrep

Single-term search for `HttpClient` across the full codebase. xray.exe token matching finds 1,072 files; rg substring matching finds 2,092 files (includes `IHttpClientFactory`, `HttpClientHandler`, etc.):

| Tool                                           | Operation                                               | Total Time | Speedup     |
| ---------------------------------------------- | ------------------------------------------------------- | ---------- | ----------- |
| `rg HttpClient -g '*.cs' -l`                   | Live file scan                                          | 32.0s      | baseline    |
| `xray grep "HttpClient" -e cs -c`            | Inverted index (total incl. load)                       | 1.76s      | **18×**     |
| ↳ index load from disk                         | LZ4 decompress + bincode deserialize (223.7 MB on disk) | 1.19s      | —           |
| ↳ search + TF-IDF rank                         | HashMap lookup + scoring                                | 0.757ms    | **42,300×** |

> **Note:** In MCP server mode, the index is loaded once at startup. All subsequent queries pay only the search+rank cost (0.6–4ms depending on hardware), not the load cost.

## CLI search latency (index pre-loaded from disk)

Measured via `xray grep` on 48,779-file C# index (754K unique tokens). Search+Rank is the pure in-memory search time; total CLI time also includes index load from disk (~1.2s, LZ4 decompress + bincode deserialize):

| Query Type                                            | Search+Rank Time | Files Matched | Notes                   |
| ----------------------------------------------------- | ---------------- | ------------- | ----------------------- |
| Single token (`OrderServiceProvider`)                 | 1.69ms           | 2,718         | rg: 2,745 files (31.2s) |
| Single token (`HttpClient`)                           | 0.76ms           | 1,072         | rg: 2,092 files (32.0s) |
| Multi-term OR (3 class variants)                      | 0.04ms           | 13            | rg: 26 files (35.3s)    |
| Multi-term AND (`IFeatureResolver` + `MonitoredTask`) | 1.01ms           | 19            | rg: 369 files (64.8s)   |
| Phrase (`new ConcurrentDictionary`)                   | 345ms            | 311           | rg: 311 files (34.4s)   |
| Regex (`I.*Cache`)                                    | 60.6ms           | 1,425         | rg: 2,650 files (33.6s) |
| Exclude filters (`StorageIndexManager`)               | 0.025ms          | 2             | rg: 4 files (22.9s)     |

**File count differences**: these CLI numbers were measured when `xray grep` still defaulted to exact token matching. The CLI now defaults to trigram substring matching (`--exact` restores the old exact-token behavior), so current CLI counts compare to rg the same way MCP counts do.

## MCP Server: xray_grep vs ripgrep (11-Test Suite)

Comprehensive comparison of MCP `tools/call` JSON-RPC queries vs `rg` (ripgrep v14.x) on the same codebase. All MCP times are in-memory (index pre-loaded at server startup); rg performs a full filesystem scan per query.

> **Note:** Tests 1–2 were measured with `substring=false` (the old default). Since `substring=true` is now the default, Tests 1 and 2 would show MCP file counts matching rg (see [File Count Differences](#file-count-differences-mcp-vs-ripgrep) for details). Tests 4–5 explicitly used `substring=true`, which is now the default behavior.

| #   | Test                                                  | MCP files | rg files   | MCP time (ms) | rg time (ms) | Speedup        |
| --- | ----------------------------------------------------- | --------- | ---------- | ------------- | ------------ | -------------- |
| 1   | Token single (`OrderServiceProvider`)                 | 2,714     | 2,741      | **1.76**      | 38,025       | **21,600×**    |
| 2   | Multi-term OR (3 variants)                            | 13        | 26         | **0.03**      | 36,921       | **1,230,700×** |
| 3   | Multi-term AND (`IFeatureResolver` + `MonitoredTask`) | 298       | 0¹         | **1.13**      | 78,717       | **69,700×**    |
| 4   | Substring compound (`FindAsyncWithQuery`)             | 3         | 3          | **1.03**      | 37,561       | **36,500×**    |
| 5   | Substring short (`ProductQuery`)                      | 28        | 28         | **0.94**      | 40,485       | **43,100×**    |
| 6   | Phrase (`new ConcurrentDictionary`)                   | 310       | 310        | **455.26**    | 39,729       | **87×**        |
| 7   | Regex (`I\w*Cache`)                                   | 1,418     | 2,642      | **131.63**    | 37,809       | **287×**       |
| 8   | Full results + context (3 lines, top 5)               | 6 files   | 415 lines  | **6.20**      | 38,590       | **6,200×**     |
| 9   | Exclude Test/Mock filters                             | 3         | 6          | **0.03**      | 27,799       | **926,600×**   |
| 10  | AST definitions + inline body                         | 18 defs   | ~798 lines | **33.20**     | 43,497       | **1,310×**     |
| 11  | Call tree (3 levels deep)                             | 48 nodes  | N/A²       | **0.49**      | N/A          | **∞**          |

> ¹ rg AND returned 0 files due to a PowerShell scripting issue with `ForEach-Object` pipeline, not a real result.
> ² `xray_callers` has no rg equivalent — it combines grep index + AST index + recursive traversal in a single 0.49ms operation. Building a 3-level call tree manually with rg would require 7+ sequential queries (estimated 5+ minutes of agent round-trips).

### Test Descriptions

#### Test 1: Token search (single term, common identifier)

- **What it tests**: Basic inverted index lookup, TF-IDF ranking
- **MCP**: `xray_grep terms=["OrderServiceProvider"] countOnly=true`
- **rg**: `rg "OrderServiceProvider" --type cs -l`

#### Test 2: Multi-term OR search (find all variants of a class)

- **What it tests**: Multi-term OR mode, ranking across variants
- **MCP**: `xray_grep terms=["UserMapperCache","IUserMapperCache","UserMapperCacheEntry"] mode="or" countOnly=true`
- **rg**: `rg "UserMapperCache|IUserMapperCache|UserMapperCacheEntry" --type cs -l`

#### Test 3: Multi-term AND search (find files using multiple types together)

- **What it tests**: AND mode intersection
- **MCP**: `xray_grep terms=["IFeatureResolver","MonitoredTask"] mode="and" countOnly=true`
- **rg**: `rg -l "IFeatureResolver" | ForEach-Object { if (rg -q "MonitoredTask" $_) { $_ } }`

#### Test 4: Substring search (compound camelCase identifier)

- **What it tests**: Trigram-based substring matching (now the default behavior)
- **MCP**: `xray_grep terms=["FindAsyncWithQuery"] countOnly=true` (substring=true is the default)
  → matched tokens: `findasyncwithqueryactivity`, `findasyncwithqueryactivityname`
- **rg**: `rg "FindAsyncWithQuery" --type cs -l`

#### Test 5: Substring search (short substring inside long identifiers)

- **What it tests**: Trigram matching for 4+ char substrings (now the default behavior)
- **MCP**: `xray_grep terms=["ProductQuery"] countOnly=true` (substring=true is the default)
  → matched 46 distinct tokens (productquerybuilder, iproductquerymanager, parsedproductqueryrequest, etc.)
- **rg**: `rg "ProductQuery" --type cs -l`

#### Test 6: Phrase search (exact multi-word sequence)

- **What it tests**: Phrase matching across adjacent tokens (requires line-by-line scan)
- **MCP**: `xray_grep terms=["new ConcurrentDictionary"] phrase=true countOnly=true`
- **rg**: `rg "new ConcurrentDictionary" --type cs -l`

#### Test 7: Regex search (pattern matching)

- **What it tests**: Regex over tokenized index
- **MCP**: `xray_grep terms=["I.*Cache"] regex=true countOnly=true`
- **rg**: `rg "I\w*Cache" --type cs -l`

#### Test 8: Full results with context lines

- **What it tests**: Line-level results, context window, ranking relevance
- **MCP**: `xray_grep terms=["InitializeIndexAsync"] showLines=true contextLines=3 maxResults=5`
- **rg**: `rg "InitializeIndexAsync" --type cs -C 3`

#### Test 9: Exclusion filters (production-only results)

- **What it tests**: Exclude patterns for Test/Mock filtering
- **MCP**: `xray_grep terms=["StorageIndexManager"] exclude=["Test","Mock"] excludeDir=["test"] countOnly=true`
- **rg**: `rg "StorageIndexManager" --type cs -l --glob "!*Test*" --glob "!*Mock*" --glob "!*test*"`

#### Test 10: AST definitions with inline source code

- **What it tests**: Tree-sitter AST index, definition lookup with inline source code
- **MCP**: `xray_definitions name=["InitializeIndexAsync"] kind=["method"] includeBody=true maxBodyLines=20`
  → Returns 18 structured definitions with signatures, parent classes, line ranges, and source code
- **rg**: `rg "InitializeIndexAsync" --type cs -A 20` (approximate, unstructured)

#### Test 11: Call tree (callers analysis)

- **What it tests**: Recursive caller tracing with depth
- **MCP**: `xray_callers method=["InitializeIndexAsync"] class="StorageIndexManager" depth=3 excludeDir=["test","Test","Mock"]`
  → Returns 48-node hierarchical call tree in ~0.5–11ms (varies by direction and graph density)
- **rg**: No equivalent. Would require 7+ sequential `rg` + `read_file` calls (estimated 5+ minutes of agent round-trips)

## File count differences: MCP vs ripgrep

> **Update:** Since the introduction of `substring=true` as the default in MCP mode, most file count mismatches between MCP and ripgrep have been eliminated. The table below documents the **historical** differences that existed when the default was exact token match, and explains why `substring=false` mode still shows different counts.

MCP and ripgrep may return different file counts for the same query when using `substring=false` (exact token mode). With the current default (`substring=true`), MCP file counts match ripgrep in most cases:

| Test       | MCP (`substring=false`) | MCP (`substring=true`, default) | rg    | Reason (when `substring=false`)                                                                | Status                                         |
| ---------- | ----------------------- | ------------------------------- | ----- | ---------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| **Test 1** | 2,714                   | ~2,741                          | 2,741 | Exact token mode misses partial matches in compound identifiers                                | ✅ Fixed — `substring=true` is now the default |
| **Test 2** | 13                      | 26                              | 26    | Exact tokens miss e.g. `UserMapperCache` inside `DeleteUserMapperCacheEntry`                   | ✅ Fixed — `substring=true` is now the default |
| **Test 3** | 298                     | 298                             | 0¹    | rg AND script has PowerShell pipeline issue; MCP AND mode works natively with set intersection | N/A (MCP is correct)                           |
| **Test 7** | 1,418                   | 1,418                           | 2,642 | MCP regex runs on tokenized index (whole tokens); rg matches raw substrings anywhere           | Expected — regex mode auto-disables substring  |
| **Test 9** | 3                       | 3                               | 6     | MCP exclude filters match more aggressively on path substrings vs rg glob patterns             | Check exclude patterns                         |

### Deep Dive: How substring search eliminates file count gaps

MCP tokenizes C# source code into **whole identifiers**. Long compound identifiers become single tokens:

```
DeleteUserMapperCacheEntryName                           → token: "deleteusermappercacheentryname"
PlatformSearchDeleteUserMapperCacheEntryActivity     → token: "platformsearchdeleteusermappercacheentryactivity"
m_userMapperCache                                        → tokens: "m", "usermappercache"
```

With **`substring=false` (exact token mode)**, searching for `UserMapperCache` only matches the token `usermappercache` — not `deleteusermappercacheentryname` (which is a different, longer token).

**Since `substring=true` is now the default**, this is no longer an issue for most users. The trigram-based substring matching automatically finds compound identifiers:

```json
// Current default behavior (substring=true): 26 files — matches rg!
{ "terms": ["UserMapperCache"], "countOnly": true }

// Exact token mode (opt-in): 13 files (misses compound identifiers)
{ "terms": ["UserMapperCache"], "substring": false, "countOnly": true }
```

Both modes complete in ~1ms. The default substring mode finds **28 matched tokens** including:
`deleteusermappercacheentryname`, `platformsearchdeleteusermappercacheentryactivity`,
`m_usermappercache`, `platformsearchusermappercacheinsertforbulkmappings_head_platformsearch_be`, etc.

**Note**: Substring mode is auto-disabled when `regex=true` or `phrase=true` is used (these modes have their own matching semantics). If you explicitly pass `substring=true` with `regex=true`, the tool returns an error to flag the conflict.

## MCP Server: xray_definitions and xray_callers

Measured via MCP `tools/call` JSON-RPC with index pre-loaded in RAM. No disk I/O on queries.

| #   | Task                                     | ripgrep (`rg`) | xray MCP | Speedup       | MCP Tool                            |
| --- | ---------------------------------------- | -------------- | ---------------- | ------------- | ----------------------------------- |
| 1   | Find a method definition by name         | 48,993 ms      | 38.7 ms          | **1,266×**    | `xray_definitions`                |
| 2   | Build a call tree (3 levels deep)        | 52,121 ms ¹    | 0.51 ms          | **~100,000×** | `xray_callers`                    |
| 3   | Find which method contains line N        | 195 ms ²       | 7.7 ms           | **25×**       | `xray_definitions` (containsLine) |
| 4   | Find all implementations of an interface | 56,222 ms      | 0.63 ms          | **~89,000×**  | `xray_definitions` (baseType)     |
| 5   | Find interfaces matching a regex         | 45,370 ms      | 58.2 ms          | **780×**      | `xray_definitions` (regex)        |
| 6   | Find classes with a specific attribute   | 38,699 ms      | 29.2 ms          | **1,325×**    | `xray_definitions` (attribute)    |

> ¹ `rg` only provides flat text search — it cannot build a call tree. The 52s is for a single `rg` query; building a 3-level tree manually would require 3–7 sequential queries totaling 150–350 seconds.
> ² For containsLine, `rg` only reads a single file (not the full repo), so the speedup is smaller.

## Performance summary by search mode

| Mode                               | Latency (MCP, in-memory) | Speedup vs rg        | Notes                                                      |
| ---------------------------------- | ------------------------ | -------------------- | ---------------------------------------------------------- |
| **Substring (trigram, default)**   | 0.9–1.7 ms               | 18,000–42,300×       | Default mode since substring=true; uses trigram index      |
| **Token (exact, substring=false)** | 0.02–1.7 ms              | 18,000–1,680,000×    | Single HashMap lookup, O(1); opt-in with `substring=false` |
| **Multi-term OR**                  | 0.04–5.6 ms              | 950,000×             | Depends on term rarity and result set size                 |
| **Multi-term AND**                 | 1.0 ms                   | 64,000×              | Set intersection                                           |
| **Phrase**                         | ~345 ms                  | 100×                 | Requires line-by-line file scan for phrase verification    |
| **Regex**                          | 61–68 ms                 | 500–555×             | Linear scan of all token keys                              |
| **Exclusion filters**              | ~0.025 ms                | 915,000×             | Path-based filtering on indexed data                       |
| **AST definitions**                | 0.6–38.7 ms              | 780–89,000×          | Depends on query type (name, baseType, regex)              |
| **AST defs + includeBody**         | ~33 ms                   | 1,310×               | Includes file I/O to read source code                      |
| **Call tree — callees (down)**     | ~0.5 ms                  | ∞ (no rg equivalent) | Pre-computed call graph traversal                          |
| **Call tree — callers (up, depth 3)** | ~3–11 ms              | ∞ (no rg equivalent) | Recursive graph walk with DI resolution                    |

**Note:** Callee traversal (direction=down) remains at ~0.5ms. Caller traversal (direction=up) is ~3–11ms due to DI resolution, test deprioritization, and popularity sorting features added since the initial benchmarks. Content search and index lookups remain stable.

### Unique capabilities (no rg equivalent)

| Capability             | Tool                 | What it does                                                                                                         |
| ---------------------- | -------------------- | -------------------------------------------------------------------------------------------------------------------- |
| **AST definitions**    | `xray_definitions` | Find classes/methods/interfaces by name, kind, parent, base type, attributes — with inline source code               |
| **Call trees**         | `xray_callers`     | Build hierarchical caller/callee trees in ~0.5ms (callees) to ~3–11ms (callers with DI resolution)                   |
| **Structured results** | `xray_grep`        | TF-IDF ranked files with occurrence counts, line numbers, context groups                                             |
| **Substring matching** | `xray_grep`        | Default `substring=true` matches inside compound identifiers (e.g., `UserMapper` finds `DeleteUserMapperCacheEntry`) |

### When to use ripgrep instead

- Searching **non-indexed file types** (XML, SQL, JSON, YAML, `.csproj`) — unless they are included in `--ext`
- Exact **raw substring** matching needed when `substring=true` behaves differently than expected (MCP tokenizes, so `m_` prefix is a separate token)
- xray MCP server is not running
- One-off searches where index build time (7–16s) is not justified

## MCP tool latency summary

Verified measurements from two machines:

| Tool                 | Query Type                             | Machine 1 (24 threads) | Machine 2 (16 threads) |
| -------------------- | -------------------------------------- | ---------------------- | ---------------------- |
| `xray_grep`        | Single token (substring=true, default) | ~1 ms                  | 1.7 ms                 |
| `xray_grep`        | Single token (substring=false)         | 0.6 ms                 | 0.8 ms                 |
| `xray_grep`        | Multi-term OR (3)                      | 5.6 ms                 | 0.06 ms                |
| `xray_grep`        | Regex (i.\*cache)                      | 44 ms                  | 340 ms                 |
| `xray_grep`        | Phrase                                 | ~345 ms                | 55 ms                  |
| `xray_grep`        | Exclusion filters                      | ~0.03 ms               | 0.04 ms                |
| `xray_grep`        | Context lines (top 5)                  | ~6 ms                  | —                      |
| `xray_definitions` | Find by name                           | 38.7 ms                | —                      |
| `xray_definitions` | Find implementations (baseType)        | 0.63 ms                | —                      |
| `xray_definitions` | containsLine                           | 7.7 ms                 | —                      |
| `xray_definitions` | Attribute filter                       | 29.2 ms                | —                      |
| `xray_definitions` | With includeBody                       | ~33 ms                 | —                      |
| `xray_callers`     | Call tree — callees (down)             | 0.5 ms                 | —                      |
| `xray_callers`     | Call tree — callers (up, depth 3)      | 3–11 ms                | —                      |

## File name search

Searching for `notepad` in 333,875 indexed entries (C:\Windows):

| Tool                                     | Operation            | Total Time |
| ---------------------------------------- | -------------------- | ---------- |
| `xray fast "notepad" -d C:\Windows -c` | Pre-built file index | 0.091s     |

Index load: 0.055s, search: 0.036s.

## Index build times

Three distinct indexes, each built independently:

| Index Type                            | What it stores                            | CLI command            | MCP tool                     |
| ------------------------------------- | ----------------------------------------- | ---------------------- | ---------------------------- |
| **FileIndex** (.file-list)            | File paths, sizes, timestamps             | `xray index`         | —                            |
| **ContentIndex** (.word-search)       | Inverted token→file map for TF-IDF search | `xray content-index` | `xray_reindex`             |
| **DefinitionIndex** (.code-structure) | AST definitions + call graph              | `xray def-index`     | `xray_reindex_definitions` |

### Build times across machines

| Index Type              | Files           | Machine 1 (24 threads) | Machine 2 (16 threads) | Disk Size (LZ4 compressed)  |
| ----------------------- | --------------- | ---------------------- | ---------------------- | --------------------------- |
| FileIndex (C:\Windows)  | 333,875 entries | ~3s                    | —                      | 47.8 MB                     |
| ContentIndex (C# files) | 48,599 files    | 7.0s                   | 15.9s                  | 223.7 MB (1.6× compression) |
| DefinitionIndex (C#)    | ~48,600 files   | 16.1s                  | 32.0s                  | 103.1 MB (3.2× compression) |

**Why is def-index 2× slower than content-index?**

- Content indexing: read file → split tokens (simple string operations)
- Definition indexing: read file → parse full AST with tree-sitter → walk AST tree → extract definitions with modifiers, attributes, base types → extract call sites from method bodies

## Search relevance evaluation

The checked relevance corpus under `benches/fixtures/relevance/` contains 18 neutral source/config/doc files and 40 graded queries across eight query classes. Every query records its intent, every top-10 result must be graded or covered by an explicit query- or corpus-level negative policy, and every graded path must be lexically reachable. Five exact-phrase queries exercise matching and occurrence-count ranking as a scorer-independent canary; the other 35 queries exercise production scorer paths, including TF-IDF and the exact single-token file-stem signal. The test-only runner calls the production `xray_grep` MCP dispatch path; it does not duplicate the scoring implementation.

Run the deterministic quality gate:

```powershell
cargo test --bin xray current_tfidf_matches_checked_relevance_baseline -- --nocapture
```

Generate a machine-local quality and latency report:

```powershell
cargo test --bin xray write_tfidf_relevance_report -- --ignored --nocapture
```

By default, the report is written to `target/relevance/tfidf-report.json` and the ready-to-review aggregate candidate to `target/relevance/tfidf-file-stem-v1-baseline-candidate.json`; the five slowest queries are included after one full warmup pass. With `XRAY_RELEVANCE_REPORT`, both files are written next to that override path. An override inside this repository must remain under `target/`; an external private path is allowed. Checked quality lives in `benches/fixtures/relevance/baseline-tfidf.json`; latency is intentionally excluded because it is machine-dependent.

For a private local corpus, keep the same JSON schema, including a non-empty `intent` on every query, and override the ignored runner inputs without adding them to Git. Corpus symlinks are rejected so indexed files and the corpus digest cannot diverge:

```powershell
$env:XRAY_RELEVANCE_SPEC = 'C:\local\relevance\queries.json'
$env:XRAY_RELEVANCE_CORPUS = 'C:\local\relevance\corpus'
$env:XRAY_RELEVANCE_REPORT = 'C:\local\relevance\tfidf-report.json'
cargo test --bin xray write_tfidf_relevance_report -- --ignored --nocapture
```

Candidate manifests use a strict query-only schema; graded relevance manifests are rejected:

```json
{
  "schemaVersion": 1,
  "corpusVersion": "local-corpus-v1",
  "extensions": ["ts"],
  "queries": [{
    "id": "partial-order-processor",
    "queryClass": "partial_identifier",
    "intent": "Locate the primary order processor implementation.",
    "request": { "terms": ["OrderProcess"], "substring": true }
  }]
}
```

Set `XRAY_RELEVANCE_CANDIDATE_SPEC`, `XRAY_RELEVANCE_CORPUS`, and `XRAY_RELEVANCE_CANDIDATES`, then run `cargo test --bin xray write_relevance_candidates -- --ignored --nocapture`. A candidate output inside this repository must remain under `target/`. Candidate reports use an independent `schemaVersion: 1`. `XRAY_RELEVANCE_SCORER` optionally selects `tfidf-file-stem-v1`, `smoothed-tfidf-file-stem-v2`, or `bm25:k1=<value>,b=<value>` for a single-run experiment. `XRAY_RELEVANCE_MODEL` is an optional provenance label for the production scorer; experimental labels are derived from their configuration and cannot be overridden. The collector calls production `xray_grep` at depth 50, sorts each candidate path set, and omits ranks and scores. Candidate pools from every model under comparison must be unioned and graded before comparing their metrics.

For test-only scorer experiments, set `XRAY_RELEVANCE_SCORERS` to a semicolon-separated matrix containing `tfidf-file-stem-v1`, `smoothed-tfidf-file-stem-v2`, or `bm25:k1=<value>,b=<value>`. Set `XRAY_RELEVANCE_CANDIDATE_DIR` and run `cargo test --bin xray write_relevance_candidate_matrix -- --ignored --nocapture` to collect candidates; after grading their union, set `XRAY_RELEVANCE_SPEC` and `XRAY_RELEVANCE_REPORT_DIR`, then run `cargo test --bin xray write_relevance_quality_matrix -- --ignored --nocapture`. The selector is compiled only for tests, scorer labels are derived from the selected configuration, and scorer-insensitive phrase or line-regex requests are rejected. Production builds continue to use `tfidf-file-stem-v1`. Offline contexts use a 1 MiB response budget so depth-50 path lists remain complete, and the harness rejects any response whose returned files do not match its reported totals. Matrix rows share one context, so their latency blocks are diagnostics rather than controlled model comparisons.

NDCG uses grades 1–3, MRR@10 treats grades 2–3 as useful, and Success@1 requires a grade-3 primary result at rank 1. Recall@50 counts every judged path (grades 1–3), but with the checked 18-file corpus it is a matching/tokenizer regression canary, not a ranking metric. ExplicitNegativeHits@10 is the total number of explicitly graded-zero results in the first 10 positions; lower is better.

Current `tfidf-file-stem-v1` baseline:

| Metric | All 40 queries | 35 production-scored queries |
| --- | ---: | ---: |
| NDCG@10 | 0.914894 | 0.926663 |
| MRR@10 | 0.987500 | 0.985714 |
| Recall@50 (matching canary) | 1.000000 | 1.000000 |
| Success@1 (grade 3) | 0.575000 | 0.657143 |
| Explicit negative hits@10 (total) | 1 | 1 |

Use the 35-query scorer aggregate when comparing TF-IDF, BM25, or boosts. The all-40 aggregate additionally guards phrase matching and occurrence-count ordering, but its five phrase queries cannot move when only token scoring changes. The generated-noise class uses distinct method-plus-type requests with method-specific judgments and grades type-only generated bindings as weak context rather than as false negatives (`NDCG@10 = 0.949760`, `Success@1 = 0.8`). The normalized file-stem signal raises the exact-identifier class to `NDCG@10 = 0.881110`, `MRR@10 = 1.0`, and `Success@1 = 0.4`; broad common terms are now the weakest production-scored class at `NDCG@10 = 0.862921`. A private large-repository corpus should remain local; only anonymized aggregate results belong in the repository.

### Locked lexical scorer experiment (2026-07-31)

A 4x4 BM25 grid was tuned only on a 65-query private corpus. The selected `bm25-file-stem-v2-k1-2-b-0.5` challenger was frozen before one comparison on a separate 39-query private holdout. Both comparisons used fully graded baseline/challenger candidate unions; frozen holdout profiles classified all 850 challenger-only paths as explicit negatives with zero uncovered candidates. The profiles define the complete positive universe, so challenger-only paths outside them are rule-graded zero and the holdout deltas are conditional on that frozen contract.

| Corpus | NDCG@10 delta | MRR@10 delta | Recall@50 delta | Success@1 delta | Explicit negatives@10 delta | `multi_term_and` NDCG delta | `multi_term_and` MRR delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Tuning corpus (65 queries) | +0.008018 | +0.019512 | +0.026423 | +0.030769 | -6 | -0.142402 | -0.160000 |
| Holdout corpus (39 queries) | +0.021110 | -0.019231 | +0.061349 | +0.128205 | +3 | -0.113853 | -0.433333 |

The challenger improved aggregate NDCG, Recall@50, and Success@1 on both corpora, but repeated the same material `multi_term_and` regression and also reduced holdout MRR@10. Production therefore remains on `tfidf-file-stem-v1`. A class-specific hybrid or another challenger requires a new tuning decision and a new locked holdout; these results do not validate one.

The schema-v3 baseline stores explicit-negative hit counts plus both query and corpus digests. Query changes, judgments, ranked top paths, any file name or bytes under the corpus root, or indexed extensions therefore require explicit baseline review. After an intentional ranking or corpus change, run the ignored report, review `target/relevance/tfidf-file-stem-v1-baseline-candidate.json`, explain every moved aggregate and per-class metric in the PR, then replace `benches/fixtures/relevance/baseline-tfidf.json` with that generated candidate. Do not hand-edit metric literals or either digest.

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

### Regex token scan

| Pattern                     | 1K files | 10K files | 50K files |
| --------------------------- | -------- | --------- | --------- |
| Broad pattern (`token_4.*`) | 2.9 µs   | 2.9 µs    | 3.1 µs    |
| Exact pattern (`class`)     | 706 ns   | 712 ns    | 776 ns    |

Regex scan time depends on number of unique tokens (500 in synthetic index), not file count.

## PERF-AUDIT-2026-04-24 baselines

The micro-benches below were added in PR `perf/00-extend-benches` as the
reference point for the PERF-AUDIT-2026-04-24 story. Each PERF-* PR
records its `before` / `after` numbers against the `pre-perf-audit`
baseline saved here.

### Running

Criterion micro-benches (PERF-01, PERF-04, PERF-05, PERF-07):

```powershell
cargo bench --bench search_benchmarks -- --save-baseline pre-perf-audit
# After each PERF-* PR:
cargo bench --bench search_benchmarks -- --baseline pre-perf-audit
```

Git-bound benches (PERF-02, PERF-03, PERF-09, PERF-04 raw spawn cost) are
measured outside criterion to avoid spawn-variance noise:

```powershell
pwsh scripts/bench-git-perf.ps1 -Repo C:\path\to\real\repo -SaveBaseline pre-perf-audit
# After each PERF-* PR:
pwsh scripts/bench-git-perf.ps1 -Repo C:\path\to\real\repo
```

### Bench-to-task mapping

| Bench group                        | Story task | What it measures                                                  |
| ---------------------------------- | ---------- | ----------------------------------------------------------------- |
| `generate_trigrams`                | PERF-05    | ASCII vs Unicode trigram tax; 18k-vocab build cost                |
| `regex_compile`                    | PERF-01    | Per-request `Regex::new` vs cached match-only                     |
| `top_authors_aggregation`          | PERF-04    | `format!` per-commit key vs tuple key over 50k synthetic commits  |
| `callers_resolve_substring_memo_shape` | PERF-07 | Per-node lookup + substring scan vs memoised resolution — measures memoisation speed-up only; substring path is a linear key walk, NOT representative of production trigram-intersection latency |
| `bench-git-perf.ps1` PERF-02       | PERF-02    | 4× sequential `git rev-parse` vs combined `for-each-ref`          |
| `bench-git-perf.ps1` PERF-03       | PERF-03    | Old (rev-parse + diff) vs new (`git show`) per-commit             |
| `bench-git-perf.ps1` PERF-09       | PERF-09    | `git blame --porcelain` subprocess only (Rust parse_blame_porcelain parser excluded — upper bound on what xray can achieve for this file) |
| `bench-git-perf.ps1` PERF-04       | PERF-04    | `git log --format='%an\|%ae' --max-count=50000` spawn + stream cost (mirrors top_authors production format) |

### Out of scope for this PR

End-to-end MCP handler latency (`xray_grep`, `xray_callers`, `xray_fast`)
is **not** covered. Adding it requires exposing `HandlerContext` /
handler entrypoints behind a `bench-internals` feature flag, which is
deferred to a follow-up story so PERF-00 stays scoped to additive,
zero-API-surface changes.

### Serialization (bincode)

Measured on 5,000-file synthetic index (15.9 MB serialized):

| Operation   | Time    |
| ----------- | ------- |
| Serialize   | 16.3 ms |
| Deserialize | 44.7 ms |

Extrapolated for real 241.7 MB index: ~700ms deserialize (matches measured 689ms load time).

> **Note:** Since Feb 2026, all index files are LZ4 frame-compressed on disk. The serialization benchmarks above measure raw bincode without compression. Actual load times include LZ4 decompression — see [Index load times](#index-load-times-measured) for compressed load measurements.

## Index load times (measured)

| Index Type             | Files   | Disk Size (LZ4) | Load Time (LZ4 decompress + deserialize) |
| ---------------------- | ------- | --------------- | ---------------------------------------- |
| ContentIndex           | 48,599  | 223.7 MB        | 1.186s                                   |
| FileIndex (C:\Windows) | 333,875 | 47.8 MB         | 0.055s                                   |
| DefinitionIndex        | ~48,600 | 103.1 MB        | 1.284s                                   |

## Comparison with ripgrep

Measured on 48,779-file C# codebase (see `docs/run-benchmarks.ps1` for automated reproduction):

| Metric                          | ripgrep | search (indexed)                      | Speedup               |
| ------------------------------- | ------- | ------------------------------------- | --------------------- |
| First query (CLI, cold)         | 32.0s   | 1.76s (incl. load)                    | **18×**               |
| Subsequent queries (MCP server) | 32.0s   | 0.02–1.7ms                            | **18,000–1,600,000×** |
| Phrase search (MCP)             | ~34s    | ~345ms                                | **100×**              |
| Regex search (MCP)              | ~34s    | 61–68ms                               | **500–555×**          |
| AST definitions (MCP)           | 39–56s  | 0.6–38.7ms                            | **780–89,000×**       |
| Call tree — callees (MCP)       | N/A     | ~0.5ms                                | ∞                     |
| Call tree — callers (MCP)       | N/A     | ~3–11ms                               | ∞                     |
| Index build (content, one-time) | N/A     | 7–16s                                 | —                     |
| Index build (defs, one-time)    | N/A     | 16–32s                                | —                     |
| Disk overhead                   | None    | ~327 MB (LZ4 compressed content+defs) | —                     |
| RAM (server mode, estimated)    | None    | ~600–800 MB (estimated, varies by repo size; e.g. ContentIndex ~350 MB + DefinitionIndex ~324 MB uncompressed in-memory for the 48,599-file C# corpus — see [storage.md](storage.md#sizes-on-disk)) | —                     |

## Bottlenecks and scaling limits

| Bottleneck              | Measured Value          | Cause                                          | Mitigation                                  |
| ----------------------- | ----------------------- | ---------------------------------------------- | ------------------------------------------- |
| Index load              | ~1.2s for 224 MB (LZ4)  | LZ4 decompression + bincode deserialization    | Memory-map + lazy load (not implemented)    |
| Phrase search           | ~345ms                  | Line-by-line file scan for phrase verification | Consider positional index (not implemented) |
| Regex search            | 61–68ms for 754K tokens | Linear scan of all keys                        | FST for prefix queries (not implemented)    |
| Multi-term OR (3 terms) | 5.6ms                   | Scoring 13K+ posting entries                   | Acceptable for interactive use              |
| Content index build     | 7.0s                    | Parallel I/O + tokenization                    | Already parallelized (24 threads)           |
| Def index build         | 16.1s                   | tree-sitter parsing CPU-bound                  | Already parallelized (24 threads)           |

## Cross-machine variability

Benchmarks measured on two machines using the same benchmark script (`run-benchmarks.ps1`). Machine 2 is an Azure VM with DevDrive (ReFS) on NVMe-backed storage:

| Metric                  | i7-12850HX (24 threads) | Xeon 8370C (16 threads) | Ratio             | Bottleneck                         |
| ----------------------- | ----------------------- | ----------------------- | ----------------- | ---------------------------------- |
| Single token search     | 1.69ms                  | 1.69ms                  | 1.0×              | CPU                                |
| Multi-term OR (3)       | 0.013ms                 | 0.063ms                 | 4.8×              | CPU                                |
| Multi-term AND (2)      | 0.034ms                 | 1.14ms                  | 33×               | CPU                                |
| Phrase search           | 345ms                   | 55ms                    | 0.16× (M2 faster) | Disk I/O                           |
| Regex (I.\*Cache)       | 61ms                    | 340ms                   | 5.6×              | CPU                                |
| HttpClient (token)      | 0.757ms                 | 0.848ms                 | 1.1×              | CPU                                |
| Live file walk          | 14.4s                   | 983ms                   | 0.07× (M2 faster) | Disk I/O                           |
| Index load (startup)    | ~1.2s                   | ~4.0s                   | 3.3×              | CPU (LZ4 decompress + deserialize) |
| Content index build     | 7.0s                    | 15.9s                   | 2.3×              | CPU + I/O                          |
| Def index build         | 16.1s                   | 32.0s                   | 2×                | CPU                                |
| Watcher update (1 file) | ~5ms (from logs)        | ~0.9s                   | 180×              | CPU (tree-sitter)                  |

**Key insight:** CPU-bound operations (regex, index deserialization, tree-sitter parsing) are 2–6× slower on the Xeon due to lower single-thread clock speed (2.80GHz vs i7 turbo 4.8GHz). I/O-bound operations (phrase verification, live file walk) are significantly faster on the Azure VM with DevDrive.

The watcher update discrepancy is notable — the original "~5ms" figure appears to have been the per-file content-only update time, while the new 0.9s measurement includes definition index re-parsing with tree-sitter (which is CPU-intensive). The true per-file update cost depends heavily on file size and CPU speed.

## Recent Optimizations (Feb 2026)

Latest `cargo bench` run (2026-02-17) shows consistent micro-optimizations across synthetic benchmarks:

| Layer              | Improvement  | Magnitude |
| ------------------ | ------------ | --------- |
| Tokenization       | Short lines  | -28.6%    |
| Tokenization       | Medium lines | -27.1%    |
| Tokenization       | Long lines   | -37.9%    |
| Tokenization       | Code blocks  | -30.0%    |
| Index lookup       | All sizes    | ~30% avg  |
| TF-IDF single term | All sizes    | ~25-30%   |
| TF-IDF multi-term  | All sizes    | ~25-36%   |
| Regex token scan   | All patterns | ~15-25%   |
| Trigram building   | All sizes    | ~18-37%   |
| Substring search   | All queries  | ~24-35%   |

**Impact:** These improvements are algorithmic micro-optimizations in CPU-bound operations (tokenization, scoring, trigram generation). End-to-end MCP query latencies (0.6–5.6ms for most queries) remain unchanged because they are dominated by:

- Hash table lookups (10ns per key, negligible impact)
- Posting list iteration (scales with result set size, not computation)
- I/O operations (context line reads, file scanning)

**Conclusion:** The codebase remains production-ready. No regressions detected. Synthetic benchmarks confirm algorithmic stability with measurable CPU efficiency gains.

## Reproducibility

All measurements in this document can be reproduced:

```bash
# Build with release optimizations
cargo build --release

# Run criterion benchmarks (synthetic, reproducible)
cargo bench

# Real-codebase benchmarks (requires indexed directory)
xray content-index -d <YOUR_DIR> -e cs

# Measure search (PowerShell)
Measure-Command { xray grep "HttpClient" -d <YOUR_DIR> -e cs -c }

# Measure ripgrep baseline
Measure-Command { rg "HttpClient" <YOUR_DIR> -g '*.cs' -l }

# Measure index build
Measure-Command { xray content-index -d <YOUR_DIR> -e cs }

# MCP benchmarks (start server, then send JSON-RPC)
xray serve --dir <YOUR_DIR> --ext cs --watch --definitions
# Paste JSON-RPC messages to stdin and measure response times

# Automated benchmark suite (PowerShell)
# Runs 9 tests comparing rg vs xray.exe CLI with real class/method names
.\docs\run-benchmarks.ps1 -SearchDir <YOUR_DIR>
```
