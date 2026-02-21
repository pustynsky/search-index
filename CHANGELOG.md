# Changelog

All notable changes to **search-index** are documented here.

Changes are grouped by date and organized into categories: **Features**, **Bug Fixes**, **Performance**, and **Internal**.

---

## 2026-02-21

### Performance

- **Optimized MCP tool descriptions for LLM token budget** — Shortened parameter descriptions across all 14 MCP tools (~100 parameters total), reducing the system prompt token footprint by ~30% (~2,000 tokens). Concrete examples moved from inline parameter descriptions to a new `parameterExamples` section in `search_help` (on-demand via 1 extra call). Critical usage hints preserved (e.g., `class` in `search_callers`). Tool-level descriptions unchanged. Semantic purpose of each parameter preserved (8-15 words). Added `test_tool_definitions_token_budget` test to prevent description bloat from re-accumulating. Added `test_render_json_has_parameter_examples` test to verify examples are accessible via `search_help`.

### Documentation

- **CLI help, LLM instructions, and documentation updated for new features** — 6 documentation changes across the codebase:
  1. `src/cli/args.rs` — Added 5 missing tools to AVAILABLE TOOLS list (`search_git_blame`, `search_branch_status`, `search_git_pickaxe`, `search_help`, `search_reindex_definitions`), bringing the list from 11 to 16 tools
  2. `src/tips.rs` — Added 3 new tips (branch status check, pickaxe usage, noCache parameter), 1 new "Code History Investigation" strategy recipe, git tools brief mention in `render_instructions()`, and `search_branch_status` in tool priority list
  3. `docs/mcp-guide.md` — Added "File Not Found Warning" section documenting the `warning` field in git tool responses when a file doesn't exist in git
  4. `docs/cli-reference.md` — Added `[GIT]` example output line to `search info` section
  5. `README.md` — Added "Branch awareness" feature mention for `branchWarning`
  6. `docs/use-cases.md` — Added "When Was This Error Introduced?" use case showing `search_branch_status` → `search_git_pickaxe` → `search_git_authors` → `search_git_diff` workflow

### Features

- **`search_git_pickaxe` MCP tool** — New tool that finds commits where specific text was added or removed using git pickaxe (`git log -S`/`-G`). Unlike `search_git_history` which shows all commits for a file, pickaxe finds exactly the commits where a given string or regex first appeared or was deleted. Supports exact text (`-S`) and regex (`-G`) modes, optional file filter, date range filters, and `maxResults` limit. Patch output truncated to 2000 chars per commit. Tool count: 16. 14 new unit tests.

- **`search_branch_status` MCP tool** — New tool that shows the current git branch status before investigating production bugs. Returns: current branch name, whether it's main/master, how far behind/ahead of remote main, uncommitted (dirty) files list, last fetch timestamp with human-readable age, and a warning if the index is built on a non-main branch or is behind remote. Fetch age warnings use thresholds: < 1 hour (none), 1–24 hours (info), 1–7 days (outdated), > 7 days (recommend fetch). Tool count: 15. 14 new unit tests (6 handler tests + 8 helper function tests).

- **`branchWarning` in index-based tool responses** — When the MCP server is started on a branch other than `main` or `master`, all index-based tool responses (`search_grep`, `search_definitions`, `search_callers`, `search_fast`) now include a `branchWarning` field in the `summary` object: `"Index is built on branch '<name>', not on main/master. Results may differ from production."` The branch is detected at server startup via `git rev-parse --abbrev-ref HEAD`. Warning is absent on `main`/`master`, when not in a git repo, or when git is unavailable. Git tools are not affected (they query git directly). 7 new unit tests.

- **Empty results validation in `search_git_history`** — When `search_git_history` returns 0 commits, the tool now checks whether the queried file is tracked by git. If the file doesn't exist in git, the response includes a `"warning"` field: `"File not found in git: <path>. Check the path."`. This helps users distinguish between "no commits in the date range" and "wrong file path". Works in both cache and CLI fallback paths. New `file_exists_in_git()` helper function. 5 new unit tests, 2 new E2E test scenarios (T70, T70b).

- **`noCache` parameter for git tools** — Added `noCache` boolean parameter to `search_git_history`, `search_git_authors`, and `search_git_activity`. When `true`, bypasses the in-memory git history cache and queries git CLI directly. Useful when cache may be stale after recent commits. Default is `false` (use cache when available). 5 new unit tests.

### Performance

- **Trigram pre-warming on server start** — Added `ContentIndex::warm_up()` method that forces all trigram index pages into resident memory after deserialization. Previously, the first 1-2 substring queries took ~3.4 seconds due to OS page faults on freshly deserialized memory. Pre-warming touches all trigram posting lists, token strings, and inverted index HashMap buckets in a background thread at server startup, eliminating the cold-start penalty without delaying server readiness. Runs after both the disk-load fast path and the background-build path. Stderr logging: `[warmup] Starting trigram pre-warm...` / `[warmup] Trigram pre-warm completed in X.Xms (N trigrams, M tokens)`. 4 new unit tests.

### Internal

- **Substring search timing instrumentation** — Added `[substring-trace]` `eprintln!` timing traces to `handle_substring_search()` in `grep.rs` for diagnosing slow cold-start substring queries (~3.4s on first 1-2 queries). Traces cover 8 stages: terms parsing, trigram dirty check + rebuild, trigram intersection (per term), token verification (`.contains()`), main index lookups, file filter checks, response JSON building, and total elapsed time. Always-on via stderr (no feature flag), does not interfere with MCP protocol on stdout. Also instruments the trigram rebuild path in `handle_search_grep()`. E2E test plan updated with T-SUBSTRING-TRACE scenario.

### Features

- **Git history cache in `search info` / `search_info`** — The `info` CLI command and MCP `search_info` tool now display `.git-history` cache files alongside existing index types (`.file-list`, `.word-search`, `.code-structure`). CLI output shows `[GIT]` entries with branch, commit count, file count, author count, HEAD hash (first 8 chars), size, and age. MCP JSON output includes `type: "git-history"` entries with full metadata. Previously, `.git-history` cache files existed on disk but were silently skipped by the info command. 4 new unit tests.

### Bug Fixes

- **File-not-found warning in `search_git_authors` and `search_git_activity`** — When these tools return 0 results and a `path`/`file` parameter was provided, they now check whether the path exists in git. If not found, the response includes `"warning": "File not found in git: <path>. Check the path."` — matching the existing behavior of `search_git_history`. Works in both cache and CLI fallback paths. 4 new unit tests.

- **7 bugs found and fixed via code review** — Comprehensive code review of `callers.rs`, `grep.rs`, and `utils.rs` found 7 bugs (2 major, 4 minor, 1 cosmetic). All fixed with tests:
  - **`is_implementation_of` dead code in production (BUG-CR-2, MAJOR)** — `verify_call_site_target()` lowercased both arguments before calling `is_implementation_of()`, which checks for uppercase `'I'` prefix — always returned false. Fuzzy DI matching (e.g., `IDataModelService` → `DataModelWebService`) never worked in the call verification path. Unit tests passed because they called the function with original-case inputs directly. **Fix:** pass original-case values from `verify_call_site_target()`. 2 new regression tests.
  - **`search_grep` ext filter single-string comparison (BUG-CR-1)** — `search_grep` compared the ext filter as a whole string (e.g., `"cs" == "cs,sql"` → false), while `search_callers` correctly split by comma. Extracted shared `matches_ext_filter()` helper. Also fixed misleading doc: schema said "(default: server's --ext)" but actual default was None. 5 new unit tests.
  - **`inject_body_into_obj` uses `read_to_string` (BUG-CR-6)** — Files with non-UTF-8 content (Windows-1252) failed body reads while the definition index was built with `read_file_lossy`. Now uses `read_file_lossy` for consistency. ~44 lossy files no longer show `bodyError`.
  - **Normal grep mode missing empty terms check (BUG-CR-7)** — `terms: ",,,"` silently returned empty results in normal mode but gave an explicit error in substring mode. Added consistent empty terms check.
  - **`maxTotalNodes: 0` returns empty tree (BUG-CR-3)** — `0 >= 0` was always true, causing immediate return. Now treats 0 as unlimited (`usize::MAX`).
  - **`direction` parameter accepts any value as "down" (BUG-CR-4)** — `"UP"`, `"sideways"`, etc. silently ran as "down". Added validation with case-insensitive comparison.
  - **Warnings array shows only first warning (BUG-CR-5, cosmetic)** — Changed from `summary["warning"]` (singular string) to `summary["warnings"]` (array) for future-proofing. **Breaking change** for consumers reading `warning` key.

- **`search_grep` substring `matchedTokens` data leak (BUG-7)** — `matchedTokens` in substring search responses was populated from the global trigram index before applying `dir`/`ext`/`exclude` filters, showing tokens from files outside the requested scope. Now `matchedTokens` only includes tokens that have at least one file passing all filters. Affects `countOnly` and full response modes.

- **Input validation hardening (6 bugs fixed)** — Systematic input validation improvements across MCP tools, found via manual fuzzing:
  - `search_definitions`: `name: ""` now treated as "no filter" instead of returning 0 results (BUG-1)
  - `search_definitions`: `containsLine: -1` now returns error instead of silently returning ALL definitions (BUG-2, most critical)
  - `search_callers`: `depth: 0` now returns error instead of empty tree (BUG-3)
  - `search_git_history`/`search_git_diff`/`search_git_activity`: reversed date range (`from > to`) now returns descriptive error instead of silently returning 0 results (BUG-4)
  - `search_fast`: `pattern: ""` now returns error instead of scanning 97K files for 0 results (BUG-5)
  - `search_grep`: `contextLines > 0` now auto-enables `showLines: true` instead of silently ignoring context (BUG-6)

- **Panic-safety in background threads** — `.write().unwrap()` on `RwLock` in `serve.rs` (4 places) replaced with `.write().unwrap_or_else(|e| e.into_inner())` to handle poisoned locks gracefully (MAJOR-1). `.join().unwrap()` on thread handles in `index.rs` and `definitions/mod.rs` replaced with `unwrap_or_else` + warning log to survive individual worker thread panics during index building (MAJOR-2).

- **Mutex `into_inner().unwrap()` → graceful recovery** — Added `recover_mutex<T>()` helper in `src/index.rs` that handles poisoned mutex with a warning log instead of panicking. Applied to 3 locations: file index build (`src/index.rs`), content index build (`src/index.rs`), and definition index build (`src/definitions/mod.rs`). Consistent with the `.lock().unwrap_or_else(|e| e.into_inner())` pattern already used for mutex lock operations throughout the codebase.

- **`format_blame_date` timezone offset not applied** — `format_blame_date()` in `src/git/mod.rs` now applies the timezone offset string (e.g., `+0300`, `-0500`, `+0545`) to the Unix timestamp before civil date calculation. Previously, the timezone string was displayed but not used in the date math, causing all blame dates to show UTC time regardless of the author's timezone. Added `parse_tz_offset()` helper. 5 new tests for timezone formatting and 9 assertions for offset parsing.

- **`next_day()` broken fallback** — The `next_day()` function in `src/git/mod.rs` previously appended `T23:59:59` to unparseable date strings, producing invalid git date arguments. Now logs a warning and returns the original date string unchanged. This path is unreachable in practice (`validate_date()` is always called first), but the fix prevents silent corruption if the code path is ever reached. 1 new test for malformed date fallback.

---

## 2026-02-20

### Features

- **Git filter by author** — Added `author` parameter to `search_git_history`, `search_git_diff`, and `search_git_activity`. Case-insensitive substring match against author name or email. Works with both cache and CLI fallback paths. Example: `"author": "alice"` returns only commits by Alice.

- **Git filter by commit message** — Added `message` parameter to `search_git_history`, `search_git_diff`, `search_git_activity`, and `search_git_authors`. Case-insensitive substring match against commit subject. Combinable with `author` and date filters. Example: `"message": "fix bug"` returns only commits with "fix bug" in the message.

- **Directory ownership in `search_git_authors`** — `search_git_authors` now accepts a `path` parameter (file or directory path, or omit for entire repo). `file` remains as a backward-compatible alias. Directory paths return aggregated authors across all files under that directory with proper commit deduplication. Omitting `path` entirely returns authors for the entire repository.

- **`search_git_blame` tool** — New MCP tool for line-level attribution via `git blame --porcelain`. Parameters: `repo` (required), `file` (required), `startLine` (optional, 1-based), `endLine` (optional). Returns commit hash (8-char short), author name, email, date (with timezone), and line content for each blamed line. Always uses CLI. Total tool count: 14.

### Internal

- **Git feature unit tests** — Added 30 new unit tests across 4 feature areas: (1) Author/message filtering for `query_file_history`, `query_authors`, `query_activity` — 18 tests covering case-insensitive author/email matching, message substring filter, combined filters, and date+author combinations; (2) Directory ownership — 1 test for whole-repo `query_authors`; (3) Git blame — 5 tests for `blame_lines()` (success, single line, nonexistent file, bad repo, content verification); (4) Blame porcelain parser — 4 tests for `parse_blame_porcelain()` (basic, repeated hash reuse, empty input) and `format_blame_date()`. Also made `parse_blame_porcelain` and `format_blame_date` `pub(crate)` for test access, fixed pre-existing tool count assertion (13→14), and updated all existing test call sites to match new 6-arg `query_file_history`, 5-arg `query_authors`, 5-arg `query_activity`, 7-arg `file_history`, 5-arg `top_authors`, 4-arg `repo_activity` signatures.

- **Git cache test coverage** — Closed 5 test coverage gaps in the git history cache module (`src/git/cache_tests.rs`): (1) integration test for `build()` with a real temp git repo (`#[ignore]`), (2) bad timestamp parsing — verifies commits with non-numeric timestamps are skipped, (3) author pool overflow boundary — verifies error at 65536 unique authors and success at 65535, (4) `cache_path_for()` different directories produce different paths, (5) E2E test in `e2e-test.ps1` for `search_git_history` cache routing. Total: 5 new unit tests + 1 E2E test.

### Bug Fixes

- **Git CLI date filtering timezone fix** — The `add_date_args()` function in `src/git/mod.rs` now appends `T00:00:00Z` to `--after`/`--before` date parameters, forcing UTC interpretation. Previously, bare `YYYY-MM-DD` dates were interpreted in the local timezone by git, causing a ±N hour mismatch with the cache path (which always uses UTC timestamps). This could cause `search_git_history` CLI fallback to miss commits at day boundaries on non-UTC systems. Affects `search_git_history`, `search_git_diff`, `search_git_authors`, and `search_git_activity` CLI paths. 23 new diagnostic unit tests added for date conversion, timestamp formatting, and cache query boundary conditions.

- **Git cache progress logging** — The git cache background thread now emits `[git-cache]` progress messages during startup and build, preventing the appearance of a "stuck" server when building the cache for large repos (3+ minutes). Messages include: initialization, branch detection, disk cache validation, build progress every 10K commits, and completion summary.

- **`search_git_authors` missing `firstChange` on cached path** — The cached code path for `search_git_authors` now correctly returns the `firstChange` timestamp instead of an empty string. Added `first_commit_timestamp` field to `AuthorSummary` in the cache module.

### Features

- **Git history cache background build + disk persistence (PR 2c)** — The git history cache is now built automatically in a background thread on server startup, saved to disk (`.git-history` file, bincode + LZ4 compressed), and loaded from disk on subsequent restarts (~100 ms vs ~59 sec full rebuild). HEAD validation detects stale caches: if HEAD matches → use disk cache; if HEAD changed (fast-forward) → rebuild; if HEAD changed (force push/rebase) → rebuild; if repo re-cloned → rebuild. Commit-graph hint emitted at startup if `.git/objects/info/commit-graph` is missing. Key changes:
  - Background thread in `serve.rs` following existing content/definition index pattern (copy-paste, no refactor)
  - `save_to_disk()` / `load_from_disk()` methods using atomic write (temp file + rename) and shared `save_compressed()`/`load_compressed()`
  - `cache_path_for()` constructs `.git-history` file path matching existing `.word-search`/`.code-structure` naming convention
  - `is_ancestor()` / `object_exists()` helpers for HEAD validation
  - `run_server()` now accepts `git_cache` and `git_cache_ready` Arc handles from `serve.rs`
  - 12 new unit tests for disk persistence, atomic write, corrupt file handling, format version validation

- **Git history cache handler integration (PR 2b)** — Integrated the git history cache into the MCP handler layer with cache-or-fallback routing. When the cache is ready (populated by background thread in PR 2c), `search_git_history`, `search_git_authors`, and `search_git_activity` use sub-millisecond cache lookups instead of 2-6 sec CLI calls. When cache is not ready, handlers transparently fall back to existing CLI code (zero regression). `search_git_diff` always uses CLI (cache has no patch data). Cache responses include `"(from cache)"` hint in the summary field. Key changes:
  - `HandlerContext` gains `git_cache: Arc<RwLock<Option<GitHistoryCache>>>` and `git_cache_ready: Arc<AtomicBool>` fields
  - Date conversion helpers: YYYY-MM-DD → Unix timestamp (Howard Hinnant algorithm) for cache query compatibility
  - Path normalization applied to `file` parameter before cache lookup
  - Response format matches CLI output exactly (same JSON structure, field names, date format)

- **Git history cache core module (PR 2a)** — Added `src/git/cache.rs` with compact in-memory cache for git history. Designed for sub-millisecond queries (vs 2-6 sec per file via CLI). Key components:
  - `GitHistoryCache` struct: compact representation (~7.6 MB for 50K commits × 65K files)
  - `CommitMeta`: 40-byte per-commit metadata with `[u8;20]` hash, i64 timestamp, u16 author index, u32 subject pool offset/length
  - Streaming parser: parses `git log --name-only` output line-by-line (no 163 MB in RAM)
  - Query API: `query_file_history()`, `query_authors()`, `query_activity()` with date filtering and path prefix matching
  - Path normalization: `\` → `/`, strip `./`, collapse `//`, `"."` → `""`
  - Serialization: `#[derive(Serialize, Deserialize)]` for reuse with existing `save_compressed()`/`load_compressed()` (bincode v1 + lz4_flex)
  - 49 unit tests covering parser, queries, normalization, edge cases, and serialization roundtrip

- **Git history tools** — 4 new MCP tools for querying git history via git CLI with in-memory cache for sub-millisecond repeat queries. Always available — no flags needed:
  - `search_git_history` — commit history for a file (hash, date, author, message)
  - `search_git_diff` — commit history with full diff/patch (truncated to ~200 lines per commit)
  - `search_git_authors` — top authors for a file ranked by commit count
  - `search_git_activity` — repo-wide activity (all changed files) for a date range

  All tools support `from`/`to`/`date` filters and `maxResults` (default: 50). Performance: ~2 sec for single file, ~8 sec for full year in a 13K-commit repo. Response truncation via existing `truncate_large_response` mechanism.

- **Code complexity metrics (`includeCodeStats`)** — `search_definitions` now computes and returns 7 code complexity metrics for methods/functions during AST indexing: cyclomatic complexity, cognitive complexity (SonarSource), max nesting depth, parameter count, return/throw count, call count (fan-out), and lambda count. Always computed when `--definitions` is used (~2-5% CPU overhead, ~7 MB RAM). Query with `includeCodeStats=true` to see metrics, or use `sortBy` (e.g., `sortBy='cognitiveComplexity'`) and `min*` filters (e.g., `minComplexity=10`, `minParams=5`) to find complex methods. Supports C# and TypeScript/TSX.

### Internal

- **Lowercase index filenames** — `sanitize_for_filename()` now lowercases all characters, producing consistent lowercase index filenames (e.g., `repos_myproject_a1b2c3d4.word-search` instead of `Repos_MyProject_a1b2c3d4.word-search`). Follows industry best practices (Cargo, npm, Docker all use lowercase). Prevents duplicate index files when the same path is referenced with different casing on case-insensitive filesystems. Old index files with uppercase names will be re-created automatically.

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

- **Git history cache documentation and cleanup (PR 2d)** — Updated all documentation (README, architecture, MCP guide, storage model, E2E test plan, changelog) to reflect the git history cache feature. Added git cache to architecture overview table, module structure, and storage format descriptions. Verified no TODO/FIXME comments in cache module. No Rust code changes.

---

## Summary

| Metric | Value |
|--------|-------|
| Total PRs | 28 |
| Features | 20 |
| Bug Fixes | 10 |
| Performance | 3 |
| Internal | 5 |
| Unit tests (latest) | 659+ |
| E2E tests (latest) | 48+ |
| Binary size reduction | 20.4 MB → 9.8 MB (−52%) |
| Index size reduction | 566 MB → 327 MB (−42%, LZ4) |
| Memory reduction | 3.7 GB → 2.1 GB (−43%) |
| Build speed improvement | 150s → 42s (3.6×) |