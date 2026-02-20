# Changelog

All notable changes to **search-index** are documented here.

Changes are grouped by date and organized into categories: **Features**, **Bug Fixes**, **Performance**, and **Internal**.

---

## 2026-02-18

### Features

- **Async MCP server startup** — server responds to `initialize` immediately; indexes are built in background threads. Tools that don't need indexes (`search_help`, `search_info`, `search_find`) work instantly. Index-dependent tools return a "building, please retry" message until ready. ([PR #17](https://github.com/pustynsky/search-index/pull/17))

- **Save indexes on graceful shutdown** — when the MCP server receives stdin close (VS Code stop), both content and definition indexes are saved to disk, preserving all incremental watcher updates across restarts. ([PR #18](https://github.com/pustynsky/search-index/pull/18))

- **Phrase search with punctuation** — `search_grep` with `phrase: true` now uses raw substring matching when the phrase contains non-alphanumeric characters (e.g., `</Property>`, `ILogger<string>`), eliminating false positives from tokenization stripping XML/code punctuation. Alphanumeric-only phrases continue to use the existing tokenized regex path. ([PR #19](https://github.com/pustynsky/search-index/pull/19))

- **TypeScript call-site extraction for `search_callers`** — `search_callers` now works for TypeScript/TSX files. Supports method calls (`this.service.getUser()`), constructor calls (`new UserService()`), static calls, `super` calls, optional chaining (`?.`), and DI constructor parameter properties. Direction `"up"` and `"down"` both supported. ([PR #11](https://github.com/pustynsky/search-index/pull/11))

- **TypeScript AST parsing** — added tree-sitter-based TypeScript/TSX definition parsing for `search_definitions`. Extracts classes, interfaces, methods, properties, fields, enums, constructors, functions, type aliases, and variables. ([PR #9](https://github.com/pustynsky/search-index/pull/9))

- **`includeBody` for `search_definitions`** — returns actual source code inline in definition results, eliminating the need for follow-up `read_file` calls. Controlled via `maxBodyLines` and `maxTotalBodyLines` parameters. ([PR #2](https://github.com/pustynsky/search-index/pull/2))

- **Substring search** — `search_grep` now supports substring matching (enabled by default). Search term `"service"` matches tokens like `userservice`, `servicehelper`, etc. Powered by trigram index for fast lookup. ([PR #3](https://github.com/pustynsky/search-index/pull/3))

- **`--metrics` CLI flag** — displays index build metrics (file count, token count, definition count, build time) when building indexes. ([PR #4](https://github.com/pustynsky/search-index/pull/4))

- **Benchmarks** — added `benches/search_benchmarks.rs` with criterion-based benchmarks for index operations. ([PR #5](https://github.com/pustynsky/search-index/pull/5))

- **LZ4 compression for index files** — all index files (`.idx`, `.cidx`, `.didx`) are now LZ4-compressed on disk, reducing total size by ~42% (566 MB → 327 MB). Backward compatible: legacy uncompressed files are auto-detected on load. ([PR #15](https://github.com/pustynsky/search-index/pull/15))

- **`search_callers` caps** — added `maxCallersPerLevel` and `maxTotalNodes` parameters to prevent output explosion for heavily-used methods. ([PR #12](https://github.com/pustynsky/search-index/pull/12))

### Bug Fixes

- **Substring AND-mode false positives** — fixed a bug where AND-mode search (`mode: "and"`) returned false positives when a single search term matched multiple tokens via the trigram index. Now tracks distinct matched term indices per file. ([PR #16](https://github.com/pustynsky/search-index/pull/16))

- **Lossy UTF-8 file reading** — files with non-UTF8 bytes (e.g., Windows-1252 `0x92` smart quotes) were silently skipped during indexing. Now uses `String::from_utf8_lossy()` with a warning log, preserving all valid content. ([PR #13](https://github.com/pustynsky/search-index/pull/13))

- **Modifier bug** — fixed definition parsing issue with C# access modifiers. ([PR #6](https://github.com/pustynsky/search-index/pull/6))

- **Code review fixes** — bounds checking, security validation for path traversal, stable hash for index file paths, underflow protection with `saturating_sub`, and monitoring improvements. ([PR #8](https://github.com/pustynsky/search-index/pull/8))

- **Version desync** — MCP protocol version now derives from `Cargo.toml` via `env!("CARGO_PKG_VERSION")` instead of a hardcoded string. ([PR #16](https://github.com/pustynsky/search-index/pull/16))

### Performance

- **Memory optimization** — eliminated forward index (~1.5 GB savings in steady-state) and added drop+reload pattern after build (~1.5 GB savings during build). Steady-state memory: ~3.7 GB → ~2.1 GB. ([PR #20](https://github.com/pustynsky/search-index/pull/20))

- **Lazy parsers + parallel tokenization** — TypeScript grammars loaded lazily (only when `.ts`/`.tsx` files are encountered); content tokenization parallelized across threads. Index build time: ~150s → ~42s (3.6× faster). ([PR #14](https://github.com/pustynsky/search-index/pull/14))

- **Eliminated ~100 MB allocation** — `reindex_definitions` response was serializing the entire index just to get its byte size. Replaced with `bincode::serialized_size()`. ([PR #16](https://github.com/pustynsky/search-index/pull/16))

### Internal

- **Module decomposition** — extracted `cli/`, `mcp/handlers/`, and other modules from monolithic `main.rs`. ([PR #7](https://github.com/pustynsky/search-index/pull/7))

- **Refactor: type safety and error handling** — introduced `SearchError` enum, eliminated duplicate type definitions, extracted `index.rs` and `error.rs` modules, fixed `total_tokens` drift in incremental updates, reduced binary size from 20.4 MB to 9.8 MB by removing incompatible SQL grammar, added 11 unit tests. ([PR #1](https://github.com/pustynsky/search-index/pull/1))

- **Tips updated** — updated MCP server system prompt instructions (`src/tips.rs`). ([PR #10](https://github.com/pustynsky/search-index/pull/10))

- **Documentation fixes** — various doc corrections and updates. ([PR #21](https://github.com/pustynsky/search-index/pull/21))

---

## Summary

| Metric | Value |
|--------|-------|
| Total PRs | 21 |
| Features | 11 |
| Bug Fixes | 5 |
| Performance | 3 |
| Internal | 4 |
| Unit tests (latest) | 280+ |
| E2E tests (latest) | 24+ |
| Binary size reduction | 20.4 MB → 9.8 MB (−52%) |
| Index size reduction | 566 MB → 327 MB (−42%, LZ4) |
| Memory reduction | 3.7 GB → 2.1 GB (−43%) |
| Build speed improvement | 150s → 42s (3.6×) |