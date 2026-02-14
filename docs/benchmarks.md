# Performance Benchmarks

All numbers in this document are **measured**, not estimated. Criterion benchmarks use synthetic data for reproducibility; CLI benchmarks use a real production codebase.

## Test Environment

| Parameter | Value                                                |
| --------- | ---------------------------------------------------- |
| CPU       | 24 logical cores                                     |
| RAM       | 32 GB                                                |
| Storage   | NVMe SSD                                             |
| OS        | Windows 11                                           |
| Rust      | 1.83+ (edition 2024)                                 |
| Build     | `--release` with LTO (`opt-level = 3`, `lto = true`) |

## Codebase Under Test

Real production C# codebase:

| Metric                       | Value              |
| ---------------------------- | ------------------ |
| Total files indexed          | 48,599             |
| File types                   | C# (.cs)           |
| Unique tokens                | 754,350            |
| Total token occurrences      | 33,082,236         |
| Definitions (AST)            | 846,167            |
| Files parsed for definitions | 53,799 (incl. SQL) |

## Content Search: search vs ripgrep

Single-term search for `HttpClient` across the full codebase (1,065 matching files, 11,706 occurrences):

| Tool                                           | Operation                         | Wall Time | Speedup     |
| ---------------------------------------------- | --------------------------------- | --------- | ----------- |
| `rg HttpClient -g '*.cs' -l`                   | Live file scan                    | 27.52s    | baseline    |
| `search find "HttpClient" --contents -e cs -c` | Live parallel walk (24 threads)   | 0.80s     | **34×**     |
| `search grep "HttpClient" -e cs -c`            | Inverted index (total incl. load) | 1.33s     | **21×**     |
| ↳ index load from disk                         | bincode deserialize 241.7 MB      | 0.689s    | —           |
| ↳ search + TF-IDF rank                         | HashMap lookup + scoring          | 0.644ms   | **42,700×** |

> **Note:** In MCP server mode, the index is loaded once at startup. All subsequent queries pay only the search+rank cost (~0.6ms), not the load cost.

## CLI Search Latency (index pre-loaded from disk)

Measured via `search grep` on 48,599-file C# index (754K unique tokens):

| Query Type                                | Search+Rank Time | Files Matched | Occurrences |
| ----------------------------------------- | ---------------- | ------------- | ----------- |
| Single token (`HttpClient`)               | 0.644ms          | 1,065         | 11,706      |
| Multi-term AND (`HttpClient,ILogger`)     | 0.500ms          | 16            | 226         |
| Multi-term OR (`HttpClient,ILogger,Task`) | 5.628ms          | 13,349        | 151,750     |
| Regex (`i.*cache` → 218 matching tokens)  | 44.24ms          | 1,419         | 4,237       |

**Note:** These times include loading index from disk (~0.69s) + search. The "Search+Rank" column is the pure in-memory search time as reported by the tool's internal timers.

## File Name Search

Searching for `notepad` in 333,875 indexed entries (C:\Windows):

| Tool                                     | Operation            | Wall Time |
| ---------------------------------------- | -------------------- | --------- |
| `search fast "notepad" -d C:\Windows -c` | Pre-built file index | 0.091s    |

Index load: 0.055s, search: 0.036s.

## Index Build Times

| Index Type                 | Files           | Threads | Build Time | Disk Size |
| -------------------------- | --------------- | ------- | ---------- | --------- |
| FileIndex (C:\Windows)     | 333,875 entries | 24      | ~3s        | 47.8 MB   |
| ContentIndex (C# files)    | 48,599 files    | 24      | 7.0s       | 241.7 MB  |
| DefinitionIndex (C# + SQL) | 53,799 files    | 24      | 16.1s      | 231.8 MB  |

**Why is def-index 2.3× slower than content-index?**

- Content indexing: read file → split tokens (simple string operations)
- Definition indexing: read file → parse full AST with tree-sitter → walk AST tree → extract definitions with modifiers, attributes, base types

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
| DefinitionIndex        | 53,799  | 231.8 MB  | ~0.7s (estimated from size parity with ContentIndex) |

## Comparison with ripgrep

| Metric                          | ripgrep | search (indexed)       | Speedup     |
| ------------------------------- | ------- | ---------------------- | ----------- |
| First query (cold)              | 27.5s   | 1.33s (incl. load)     | **21×**     |
| Subsequent queries (MCP server) | 27.5s   | 0.6ms                  | **45,000×** |
| Index build (one-time)          | N/A     | 7.0s                   | —           |
| Disk overhead                   | None    | 241.7 MB               | —           |
| RAM (server mode, estimated)    | None    | ~400 MB (not measured) | —           |

## Bottlenecks and Scaling Limits

| Bottleneck              | Measured Value       | Cause                                | Mitigation                               |
| ----------------------- | -------------------- | ------------------------------------ | ---------------------------------------- |
| Index load              | 689ms for 242 MB     | bincode deserialization + allocation | Memory-map + lazy load (not implemented) |
| Regex search            | 44ms for 754K tokens | Linear scan of all keys              | FST for prefix queries (not implemented) |
| Multi-term OR (3 terms) | 5.6ms                | Scoring 13K+ posting entries         | Acceptable for interactive use           |
| Content index build     | 7.0s                 | Parallel I/O + tokenization          | Already parallelized (24 threads)        |
| Def index build         | 16.1s                | tree-sitter parsing CPU-bound        | Already parallelized (24 threads)        |

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
```
