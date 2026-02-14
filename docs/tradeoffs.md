# Design Trade-offs

Every architectural decision has alternatives. This document captures what was chosen, what was rejected, and why.

## 1. Index Storage: Bincode vs Alternatives

### Chosen: Bincode (binary serialization)

**Why:**

- Zero-config — serialize any Rust struct with `#[derive(Serialize, Deserialize)]`
- Fast — near-zero overhead deserialization, close to raw memory layout
- Single-file — each index is one `.idx`/`.cidx`/`.didx` file
- No runtime dependencies — no database process, no WAL, no compaction

**Rejected alternatives:**

| Alternative                   | Why Not                                                                                                                        |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| **SQLite**                    | Adds 1MB+ dependency, slower for bulk reads (entire index loaded at once), row-level access unnecessary for our access pattern |
| **RocksDB**                   | C++ dependency, complex build, designed for incremental writes — overkill for batch-build-then-read pattern                    |
| **Cap'n Proto / FlatBuffers** | Zero-copy is appealing but requires schema files, more complex API, marginal gain when entire index fits in RAM                |
| **JSON**                      | 5-10x larger on disk, 10-50x slower to parse for large indexes                                                                 |
| **MessagePack**               | Similar to bincode but less Rust-native, no meaningful advantage                                                               |

**Known limitations:**

- Bincode format is not stable across major versions — index files are not portable between bincode 1.x and 2.x
- No incremental writes — entire index must be serialized/deserialized atomically
- No memory-mapped I/O — full deserialization into heap on load

**When to reconsider:** If indexes exceed available RAM (>4GB), a memory-mapped approach (e.g., FST for the token map + mmap'd postings) would be necessary.

## 2. Inverted Index: HashMap vs FST/Trie

### Chosen: `HashMap<String, Vec<Posting>>`

**Why:**

- O(1) exact token lookup — the primary query pattern
- Simple to implement incremental updates (insert/remove postings)
- Rust's `HashMap` is high-performance (SwissTable implementation since 1.36)
- Easy to serialize with bincode

**Rejected alternatives:**

| Alternative         | Why Not                                                                                                                                                                                                                 |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **FST (fst crate)** | Excellent for prefix/range queries and memory efficiency, but immutable — cannot do incremental updates without full rebuild. Tantivy uses FST, but they have a segment-merge architecture we don't need at this scale. |
| **Trie**            | Better for prefix matching, but higher memory overhead per node, slower for exact lookups, complex to serialize                                                                                                         |
| **BTreeMap**        | Sorted iteration is unnecessary for our queries, 2-3x slower than HashMap for exact lookups                                                                                                                             |
| **Tantivy**         | Full-featured search engine — adds 10MB+ to binary, brings its own segment management, query parser, etc. Overkill for single-directory code search.                                                                    |

**Known limitations:**

- No prefix/fuzzy search without scanning all keys (regex mode does this, measured 44ms for 754K tokens)
- Memory usage is O(unique_tokens × avg_posting_size) — for 49K files this is 242MB on disk
- Hash collisions can degrade under adversarial inputs (not a concern for code search)

**When to reconsider:** If we add fuzzy/typo-tolerant search, an FST or Levenshtein automaton would be much more efficient than regex scanning all keys.

## 3. Ranking: TF-IDF vs BM25

### Chosen: Classic TF-IDF

```
score = (occurrences / file_token_count) × ln(total_files / files_with_term)
```

**Why:**

- Simple — one formula, no tunable parameters
- Effective for code search — code is more structured than natural language, simple TF-IDF works well
- Fast — single pass over postings, no normalization constants to precompute
- Predictable — developers can reason about why a result ranks higher

**Rejected alternatives:**

| Alternative         | Why Not                                                                                                                                                                                                                                                                                      |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **BM25**            | Adds two tunable parameters (k1, b) that require corpus-specific tuning. Marginal improvement for code search where documents (files) have similar structure. BM25's document length normalization helps with variable-length prose documents but code files are already relatively uniform. |
| **PageRank-style**  | Would require call graph analysis. Expensive to compute, unclear benefit for code search.                                                                                                                                                                                                    |
| **Embedding-based** | Requires ML model, GPU/large CPU, ~100x slower per query. Out of scope for a CLI tool.                                                                                                                                                                                                       |

**Known limitations:**

- No field boosting — a match in class name vs. method body has equal weight
- No position proximity — `HttpClient` on line 1 and line 500 contribute equally
- TF normalization by file size means a 10-line file mentioning `HttpClient` once will rank above a 1000-line file mentioning it 5 times

**When to reconsider:** If user feedback shows ranking quality issues, BM25 with default parameters (k1=1.2, b=0.75) would be a minimal-effort upgrade.

## 4. Concurrency: RwLock vs Lock-Free

### Chosen: `Arc<RwLock<ContentIndex>>`

**Why:**

- Simple correctness — Rust's type system enforces exclusive writes
- Appropriate for the access pattern: many reads (search queries), rare writes (watcher updates)
- `RwLock` allows concurrent reads with no contention
- Single writer (watcher thread) means no write contention

**Rejected alternatives:**

| Alternative                                | Why Not                                                                                                                                                             |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Lock-free (crossbeam SkipMap, dashmap)** | Adds dependency, more complex code, marginal benefit — we have exactly 1 writer and writes are infrequent (debounced to every 500ms). Lock contention is near-zero. |
| **Copy-on-write (Arc swap)**               | Would require cloning the entire index on update (~400MB). Only viable with an immutable/persistent data structure.                                                 |
| **Actor model (channels)**                 | Adds complexity. The MCP server is single-threaded on stdin, so actor model doesn't provide concurrency benefit.                                                    |
| **No locking (single-threaded)**           | Not possible — watcher runs on a separate OS thread by design (notify crate requirement).                                                                           |

**Known limitations:**

- Writer starvation is theoretically possible if search queries are continuous, but MCP queries are human-driven (~1/sec max) so this doesn't happen in practice
- `RwLock` on Windows is not fair — but our usage pattern (rare writes) makes this irrelevant

## 5. Tree-sitter vs Regex for Code Parsing

### Chosen: tree-sitter AST parsing

**Why:**

- Full syntactic understanding — correctly handles nested classes, partial classes, multi-line signatures
- Modifiers, attributes, base types extracted as structured data
- Line range tracking enables `containsLine` queries (which method is on line N?)
- Language grammar maintained by the community, handles edge cases we'd never cover with regex

**Rejected alternatives:**

| Alternative                        | Why Not                                                                                                                                                |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Regex patterns**                 | Cannot handle nesting, multi-line constructs, or distinguish between definition and usage. Would miss partial classes, expression-bodied members, etc. |
| **LSP (Language Server Protocol)** | Requires running the actual language server (Roslyn for C#). 10-100x slower, requires .NET SDK installed, heavy process.                               |
| **ctags/universal-ctags**          | External tool dependency. Less structured output. Cannot extract attributes, base types, or modifiers.                                                 |
| **syn (Rust AST)**                 | Only works for Rust. We need C# and SQL.                                                                                                               |

**Known limitations:**

- tree-sitter grammars are large (C# grammar adds ~2MB to binary)
- Adding a new language requires a new tree-sitter grammar crate + parser implementation (~200 LOC per language)
- SQL grammar (tree-sitter-sequel-tsql) may not cover all T-SQL dialects perfectly

## 6. Tokenization: Simple Split vs Language-Aware

### Chosen: Character-class split + lowercase

```rust
line.split(|c: char| !c.is_alphanumeric() && c != '_')
    .filter(|s| s.len() >= min_len)
    .map(|s| s.to_lowercase())
```

**Why:**

- Language-agnostic — works for C#, SQL, Rust, Python, JavaScript, prose
- Fast — no regex, no Unicode normalization, single pass
- Predictable — developers know exactly what tokens are indexed
- Preserves underscores — `_client` stays as one token
- Case-insensitive via lowercase — `HttpClient` and `httpclient` are the same token

**Rejected alternatives:**

| Alternative                | Why Not                                                                                                                                                                |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **camelCase splitting**    | `HttpClient` → `http`, `client` would enable partial word matching but causes false positives. `HttpClient` would match searches for just `client`.                    |
| **N-gram**                 | Enables fuzzy matching but massively increases index size (3-gram of `HttpClient` = 8 grams). Not worth the trade-off for code search where exact tokens are the norm. |
| **Stemming/lemmatization** | Designed for natural language. Code identifiers don't follow natural language morphology. `async` should not match `asynchronous`.                                     |
| **Unicode-aware (ICU)**    | Adds heavy dependency. Code identifiers are ASCII in >99% of codebases.                                                                                                |

**Known limitations:**

- `HttpClient` becomes one token `httpclient` — cannot search for files using specifically `Http` but not `HttpClient`
- Numbers are included: `int32` is one token. This is usually desirable for code.
- Very short tokens (1 char) are excluded by default (min_len=2)

## 7. MCP Transport: Stdio vs HTTP

### Chosen: Stdio (stdin/stdout)

**Why:**

- Zero configuration — no port conflicts, no firewall rules, no TLS
- Lowest latency — direct pipe, no TCP overhead
- VS Code native — built-in process spawning, no external server to manage
- Security — no network exposure, process isolation by OS

**Rejected alternatives:**

| Alternative   | Why Not                                                                                                                                     |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| **HTTP/SSE**  | Network overhead, requires port management, firewall issues in corporate environments. VS Code MCP spec supports both but stdio is simpler. |
| **WebSocket** | Same issues as HTTP plus connection management complexity.                                                                                  |
| **gRPC**      | Adds protobuf dependency, code generation step. Overkill for single-client, low-QPS scenario.                                               |

**Known limitations:**

- Single client — only one process can read stdin at a time
- No remote access — must run on same machine as the AI agent
- Debugging requires stderr logging — cannot use stdout for diagnostics

## Summary Matrix

| Decision        | Chosen                 | Key Reason                  | Reconsider When              |
| --------------- | ---------------------- | --------------------------- | ---------------------------- |
| Storage         | Bincode                | Fast, simple, single-file   | Index > 4GB RAM              |
| Index structure | HashMap                | O(1) lookup, easy updates   | Need fuzzy search            |
| Ranking         | TF-IDF                 | Simple, effective for code  | Ranking quality complaints   |
| Concurrency     | RwLock                 | Correct, minimal contention | High-throughput multi-client |
| Code parsing    | tree-sitter            | Full AST, structured output | n/a (correct choice)         |
| Tokenization    | char-split + lowercase | Fast, language-agnostic     | Need camelCase/fuzzy         |
| Transport       | Stdio                  | Zero-config, lowest latency | Need remote access           |
