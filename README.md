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
| [CLI Reference](docs/cli-reference.md) | All commands with examples and options |
| [MCP Server Guide](docs/mcp-guide.md) | Setup, tools API, JSON-RPC examples |
| [Architecture](docs/architecture.md) | System overview, component design, data flow diagrams |
| [Storage Model](docs/storage.md) | Index formats, serialization, staleness, incremental updates |
| [Concurrency](docs/concurrency.md) | Thread model, lock strategy, watcher design |
| [Trade-offs](docs/tradeoffs.md) | Design decisions with alternatives considered |
| [Benchmarks](docs/benchmarks.md) | Performance data, scaling estimates, industry comparison |
| [E2E Test Plan](docs/e2e-test-plan.md) | 40+ end-to-end test cases (24 CLI + 16 MCP) with automation script |

## Features

- **Parallel filesystem walk** — uses all available CPU cores for maximum throughput
- **File name index** — pre-built index for instant file lookups (like [Everything](https://www.voidtools.com/))
- **Inverted content index** — language-agnostic tokenizer maps tokens to files for instant full-text search across any text file (like Elasticsearch)
- **TF-IDF ranking** — results sorted by relevance, most relevant files first
- **Regex support** — full Rust regex syntax for pattern matching
- **Respects `.gitignore`** — automatically skips ignored files
- **Extension filtering** — limit search to specific file types
- **MCP Server** — native Model Context Protocol server for AI agents (VS Code Roo, Copilot, Claude) with async startup
- **Code definition index** — tree-sitter AST parsing for structural code search *(C# and TypeScript/TSX)*
- **Parallel tokenization** — content index tokenization parallelized across all CPU cores
- **Parallel parsing** — multi-threaded tree-sitter parsing with lazy grammar loading
- **File watcher** — incremental index updates on file changes (<1s per file)
- **Substring search** — trigram-indexed substring matching within tokens (~0.07ms vs ~44ms for regex)
- **LZ4 index compression** — all index files compressed on disk with backward-compatible loading

## Quick Start

### Installation

```bash
git clone <repo-url>
cd search
cargo build --release
```

Requires [Rust](https://rustup.rs/) 1.70+. Binary: `target/release/search.exe` (Windows) or `target/release/search` (Linux/Mac).

### CLI Usage

```bash
# Build content index for C# files
search content-index -d C:\Projects -e cs

# Search by token (TF-IDF ranked)
search grep "HttpClient" -d C:\Projects -e cs

# Search file names (instant)
search fast "UserService" -d C:\Projects -e cs

# Live filesystem search (no index needed)
search find "TODO" -d C:\Projects --contents -e cs
```

See [CLI Reference](docs/cli-reference.md) for all commands and options.

### MCP Server (AI Agent Integration)

```bash
# Start MCP server with file watching and code definitions
search serve --dir C:\Projects --ext cs --watch --definitions
```

See [MCP Server Guide](docs/mcp-guide.md) for VS Code setup, tools API, and examples.

## Architecture Overview

The engine uses three independent index types:

| Index | File | Created by | Searched by | Stores |
|---|---|---|---|---|
| File name | `.idx` | `search index` | `search fast` | File paths, sizes, timestamps |
| Content | `.cidx` | `search content-index` | `search grep` | Token → (file, line numbers) map |
| Definitions | `.didx` | `search def-index` | `search_definitions` / `search_callers` | AST-extracted classes, methods, call sites |

Indexes are stored in `%LOCALAPPDATA%\search-index\` and are language-agnostic for content search, language-specific (C#, TypeScript/TSX) for definitions. See [Architecture](docs/architecture.md) for details.

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

# Run benchmarks
cargo bench
```

| Category | Tests |
|---|---|
| Unit tests | `clean_path`, `tokenize`, staleness, serialization roundtrips, TF-IDF ranking |
| Integration | Build + search ContentIndex, build FileIndex, MCP server end-to-end |
| MCP Protocol | JSON-RPC parsing, initialize, tools/list, tools/call, notifications, errors |
| Substring/Trigram | Trigram generation, index build, substring search, 13 e2e integration tests |
| Definitions | C# parsing, TypeScript/TSX parsing, SQL parsing, incremental update |
| Property tests | Tokenizer invariants, posting roundtrip, index consistency, TF-IDF ordering |
| Benchmarks | Tokenizer throughput, index lookup latency, TF-IDF scoring, regex scan |

## License

MIT