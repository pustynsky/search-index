# E2E Test Plan — Search Engine

## Overview

This document defines end-to-end tests for the `search` binary. These tests exercise
real CLI commands against a real directory to verify the full pipeline: indexing, searching,
output format, and all feature flags (including substring search via trigram index).

**Run these tests after every major refactoring, before merging PRs, and after dependency upgrades.**

> **Note:** MCP `search_grep` defaults to `substring: true` since v0.2. Tests that expect exact-token behavior must pass `substring: false` explicitly.

## Configuration

| Variable   | Default              | Description                                                    |
| ---------- | -------------------- | -------------------------------------------------------------- |
| `TEST_DIR` | `.` (workspace root) | Directory to index and search                                  |
| `TEST_EXT` | `rs`                 | File extension to index                                        |
| `BINARY`   | `cargo run --`       | Path to the binary (use `./target/release/search` for release) |

To run against a different directory:

```powershell
$env:TEST_DIR = "C:\Projects\MyApp"
$env:TEST_EXT = "cs"
```

## Prerequisites

```powershell
# Build the binary
cargo build

# Ensure unit tests pass first
cargo test
```

---

## Test Cases

### T01: `find` — Live filesystem search (file names)

**Command:**

```powershell
cargo run -- find "main" -d $TEST_DIR -e $TEST_EXT
```

**Expected:**

- Exit code: 0
- stdout: at least 1 file path containing "main"
- stderr: summary line with `N matches found among M entries in X.XXXs`

**Validates:** Live filesystem walk, file name matching, extension filter.

---

### T02: `find` — Content search

**Command:**

```powershell
cargo run -- find "fn main" -d $TEST_DIR -e $TEST_EXT --contents
```

**Expected:**

- Exit code: 0
- stdout: at least 1 line in format `path:line: content`
- stderr: summary with match count

**Validates:** Content search mode, line-level matching.

---

### T03: `find` — Regex mode

**Command:**

```powershell
cargo run -- find "fn\s+\w+" -d $TEST_DIR -e $TEST_EXT --contents --regex
```

**Expected:**

- Exit code: 0
- stdout: matching lines with function definitions
- stderr: summary

**Validates:** Regex pattern compilation and matching.

---

### T04: `find` — Case-insensitive search

**Command:**

```powershell
cargo run -- find "CONTENTINDEX" -d $TEST_DIR -e $TEST_EXT --contents -i
```

**Expected:**

- Exit code: 0
- stdout: lines containing "ContentIndex" (original case)

**Validates:** Case-insensitive flag.

---

### T05: `find` — Count-only mode

**Command:**

```powershell
cargo run -- find "fn" -d $TEST_DIR -e $TEST_EXT --contents -c
```

**Expected:**

- Exit code: 0
- stdout: empty (no file paths printed)
- stderr: `N matches found among M entries`

**Validates:** Count-only flag suppresses output.

---

### T06: `index` — Build file index

**Command:**

```powershell
cargo run -- index -d $TEST_DIR
```

**Expected:**

- Exit code: 0
- stderr: `Indexing ...`, `Indexed N entries in X.XXXs`, `Index saved to ... (X.X MB)`
- A `.idx` file created in the index directory

**Validates:** File index build and persistence.

---

### T07: `fast` — Search file name index

**Command:**

```powershell
cargo run -- fast "main" -d $TEST_DIR -e $TEST_EXT
```

**Expected:**

- Exit code: 0
- stdout: at least 1 file path
- stderr: match count, index load/search timing

**Validates:** File index loading + search. Auto-builds index if missing.

---

### T08: `fast` — Regex + case-insensitive

**Command:**

```powershell
cargo run -- fast ".*handler.*" -d $TEST_DIR -e $TEST_EXT --regex -i
```

**Expected:**

- Exit code: 0
- stdout: file paths matching the pattern

**Validates:** Regex and ignore-case in fast search.

---

### T09: `fast` — Dirs-only / files-only filters

**Command:**

```powershell
cargo run -- fast "" -d $TEST_DIR --dirs-only
cargo run -- fast "" -d $TEST_DIR --files-only
```

**Expected:**

- `--dirs-only`: only `[DIR]` entries
- `--files-only`: no `[DIR]` entries

**Validates:** Type filtering.

---

### T09a: `fast` — Comma-separated multi-term search (OR logic)

**Command:**

```powershell
cargo run -- fast "main,lib,handler" -d $TEST_DIR -e $TEST_EXT
```

**Expected:**

- Exit code: 0
- stdout: file paths matching ANY of the comma-separated terms (e.g., files containing "main", "lib", or "handler" in their name)
- Returns more results than searching for a single term

**Validates:** Comma-separated patterns are split and matched with OR logic. Each term is matched independently as a substring of the file name.

---

### T09b: `fast` — Comma-separated multi-term search via MCP `search_fast`

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_fast","arguments":{"pattern":"main,lib,handler","ext":"rs"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC response with `"result"` containing search results
- `summary.totalMatches` > 1 (matches files containing ANY of the terms)
- `files` array contains paths matching "main", "lib", or "handler"

**Validates:** MCP `search_fast` tool supports comma-separated multi-term OR search.

---

### T10: `content-index` — Build content index

**Command:**

```powershell
cargo run -- content-index -d $TEST_DIR -e $TEST_EXT
```

**Expected:**

- Exit code: 0
- stderr: `Building content index...`, `Indexed N files, M unique tokens (T total) in X.XXXs`
- stderr: `Content index saved to ... (X.X MB)`
- A `.cidx` file created in the index directory

**Validates:** Content index build, tokenization, persistence.

---

### T11: `grep` — Single term search

**Command:**

```powershell
cargo run -- grep "tokenize" -d $TEST_DIR -e $TEST_EXT
```

**Expected:**

- Exit code: 0
- stdout: TF-IDF ranked file list with scores, occurrences, lines
- stderr: summary with file count, token count, timing

**Validates:** Inverted index lookup, TF-IDF scoring, ranking.

---

### T12: `grep` — Multi-term OR

**Command:**

```powershell
cargo run -- grep "tokenize,posting" -d $TEST_DIR -e $TEST_EXT
```

**Expected:**

- Exit code: 0
- stdout: files containing EITHER term, `terms_matched` shows `1/2` or `2/2`
- stderr: `[OR]` mode indicated

**Validates:** Comma-separated OR search.

---

### T13: `grep` — Multi-term AND

**Command:**

```powershell
cargo run -- grep "tokenize,posting" -d $TEST_DIR -e $TEST_EXT --all
```

**Expected:**

- Exit code: 0
- stdout: only files containing BOTH terms (fewer results than T12)
- All results show `2/2` terms matched
- stderr: `[AND]` mode indicated

**Validates:** AND mode filtering.

---

### T14: `grep` — Regex token matching

**Command:**

```powershell
cargo run -- grep ".*stale.*" -d $TEST_DIR -e $TEST_EXT --regex
```

**Expected:**

- Exit code: 0
- stderr: `Regex '...' matched N tokens`
- stdout: files containing tokens matching the pattern

**Validates:** Regex expansion against index keys.

---

### T15: `grep` — Phrase search

**Command:**

```powershell
cargo run -- grep "pub fn" -d $TEST_DIR -e $TEST_EXT --phrase --show-lines
```

**Expected:**

- Exit code: 0
- stderr: `Phrase search: ...` with token list and regex
- stdout: matching lines showing `pub fn` as exact phrase

**Validates:** Phrase tokenization, AND candidate narrowing, regex verification.

---

### T16: `grep` — Show lines with context

**Command:**

```powershell
cargo run -- grep "is_stale" -d $TEST_DIR -e $TEST_EXT --show-lines -C 2 --max-results 2
```

**Expected:**

- Exit code: 0
- stdout: matching lines marked with `>`, context lines marked with ` `, separators `--`
- At most 2 files shown
- Each match has 2 lines before and 2 lines after

**Validates:** Context lines, max-results truncation, match markers.

---

### T17: `grep` — Exclude dir / exclude pattern

**Command:**

```powershell
cargo run -- grep "ContentIndex" -d $TEST_DIR -e $TEST_EXT --exclude-dir bench --exclude test
```

**Expected:**

- Exit code: 0
- stdout: no paths containing "bench" or "test" (case-insensitive)
- Fewer results than unfiltered T11

**Validates:** Exclusion filters.

---

### T18: `grep` — Count-only mode

**Command:**

```powershell
cargo run -- grep "fn" -d $TEST_DIR -e $TEST_EXT -c
```

**Expected:**

- Exit code: 0
- stdout: empty (no file list)
- stderr: `N files, M occurrences matching...`

**Validates:** Count-only suppresses file output.

---

### T19: `info` — Show all indexes

**Command:**

```powershell
cargo run -- info
```

**Expected:**

- Exit code: 0
- stderr: `Index directory: ...`
- stdout: list of `[FILE]` and `[CONTENT]` entries with age, size, staleness

**Validates:** Index discovery, deserialization of all index types.

---

### T19a: `cleanup` — Remove orphaned index files

**Setup:**

```powershell
# Create a temp directory, index it, then delete the directory
$tmp = New-Item -ItemType Directory -Path "$env:TEMP\search_cleanup_test_$(Get-Random)"
cargo run -- index -d $tmp
Remove-Item -Recurse -Force $tmp
```

**Command:**

```powershell
cargo run -- cleanup
```

**Expected:**

- Exit code: 0
- stderr: `Scanning for orphaned indexes in ...`
- stderr: `Removed orphaned index: ... (root: ...search_cleanup_test...)`
- stderr: `Removed N orphaned index file(s).`
- After cleanup, `search info` should NOT list the deleted temp directory

**Validates:** Orphaned index detection, safe removal, root field extraction from binary index files.

---

### T20: `def-index` — Build definition index

**Command:**

```powershell
cargo run -- def-index -d $TEST_DIR -e $TEST_EXT
```

**Expected:**

- Exit code: 0
- stderr: `[def-index] Found N files to parse`
- stderr: `[def-index] Parsed N files in X.Xs, extracted M definitions`
- A `.didx` file created

**Validates:** Tree-sitter parsing, definition extraction, persistence.

**Note:** For `.rs` files, 0 definitions is expected (parser supports C# and TypeScript/TSX only).
For C# or TypeScript projects, expect hundreds/thousands of definitions.
SQL parsing is currently disabled.

---

### T21: `grep` — Invalid regex error handling

**Command:**

```powershell
cargo run -- grep "[invalid" -d $TEST_DIR -e $TEST_EXT --regex
```

**Expected:**

- Exit code: 1
- stderr: `Invalid regex '[invalid': ...`

**Validates:** Graceful error on bad regex.

---

### T22: `find` — Nonexistent directory

**Command:**

```powershell
cargo run -- find "test" -d /nonexistent/path
```

**Expected:**

- Exit code: 1
- stderr: `Directory does not exist: /nonexistent/path`

**Validates:** Graceful error on missing directory.

---

### T23: `grep` — No index available

**Command:**

```powershell
cargo run -- grep "test" -d /tmp/empty_dir_no_index -e xyz
```

**Expected:**

- Exit code: 1
- stderr: `No content index found for ...`

**Validates:** Graceful error when no index exists.

---

### T24: `grep` — Before/After context lines

**Command:**

```powershell
cargo run -- grep "is_stale" -d $TEST_DIR -e $TEST_EXT --show-lines -B 1 -A 3
```

**Expected:**

- Exit code: 0
- 1 line before each match, 3 lines after
- Match lines marked with `>`

**Validates:** Asymmetric context (-B/-A) vs symmetric (-C).

---

### T25: `serve` — MCP server starts and responds to initialize

**Command:**

```powershell
$init = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}'
echo $init | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC response with `"result"` containing `"serverInfo"` and `"capabilities"`
- Response includes `"tools"` capability

**Validates:** MCP server startup, JSON-RPC initialize handshake.

---

### T26: `serve` — MCP tools/list returns all tools

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC response with 9 tools: `search_grep`, `search_find`, `search_fast`, `search_info`, `search_reindex`, `search_reindex_definitions`, `search_definitions`, `search_callers`, `search_help`
- Each tool has `name`, `description`, `inputSchema`
- `search_definitions` inputSchema includes `includeBody` (boolean), `maxBodyLines` (integer), and `maxTotalBodyLines` (integer) parameters

**Validates:** Tool discovery, tool schema generation, `search_definitions` schema includes body-related parameters.

---

### T27: `serve` — MCP search_grep via tools/call

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"tokenize"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC response with `"result"` containing search results
- Result content includes `files` array and `summary` object
- `summary.totalFiles` > 0

**Validates:** MCP tool dispatch, search_grep handler, JSON-RPC tools/call.

---

### T27a: `serve` — search_grep with `showLines: true` (compact grouped format)

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"<some_known_token>","showLines":true,"contextLines":2,"maxResults":1}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC response with search results
- Each file object contains a `"lineContent"` array
- Each element in `lineContent` is a group with:
  - `"startLine"` (integer, 1-based) — first line number in the group
  - `"lines"` (string array) — source code lines in order
  - `"matchIndices"` (integer array, 0-based, optional) — indices within `lines` where matches occur
- Groups are separated when there are gaps in line numbers
- No old-format fields (`line`, `text`, `isMatch`) are present

**Validates:** `showLines` returns compact grouped format with `startLine`, `lines[]`, and `matchIndices[]`. Context lines appear around matches.

**Note:** Replace `<some_known_token>` with a token that exists in the indexed codebase.

---

### T27b: `serve` — search_grep phrase search with `showLines: true` (compact grouped format)

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"<some_known_phrase>","phrase":true,"showLines":true,"contextLines":1,"maxResults":1}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC response with search results
- Each file object contains a `"lineContent"` array with compact grouped format (same as T27a)
- Phrase search code path produces identical format to token search path

**Validates:** Phrase search path also uses compact grouped `lineContent` format (both code paths produce consistent output).

**Note:** Replace `<some_known_phrase>` with an exact phrase that exists in the indexed codebase.

---

### T28: `serve` — MCP search_definitions (requires --definitions)

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"name":"tokenize"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT --definitions
```

**Expected:**

- stdout: JSON-RPC response with definition results
- For Rust codebase: 0 results (tree-sitter supports C#/TypeScript/SQL only)
- For C# or TypeScript codebase: results with `name`, `kind`, `file`, `lines`

**Validates:** search_definitions handler, definition index loading, AST-based search.

**Note:** Requires `--definitions` flag. For `.rs` files, 0 results is expected. For TypeScript files, definition kinds include `function`, `typeAlias`, `variable`, etc.

---

### T28a: `serve` — search_definitions with `includeBody: true`

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"name":"<some_known_def>","includeBody":true}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT --definitions
```

**Expected:**

- stdout: JSON-RPC response with definition results
- Each definition object contains a `"bodyStartLine"` (integer, 1-based) and `"body"` array field (string array of source lines)
- `summary` object includes `"totalBodyLinesReturned"` field

**Validates:** `includeBody` flag causes body content to be returned alongside definitions.

**Note:** Replace `<some_known_def>` with a definition name that exists in the indexed codebase.

---

### T28b: `serve` — search_definitions with `includeBody: true, maxBodyLines: 5`

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"name":"<some_known_long_def>","includeBody":true,"maxBodyLines":5}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT --definitions
```

**Expected:**

- stdout: JSON-RPC response with definition results
- Each definition's `"body"` array has at most 5 entries
- If a definition is longer than 5 lines: `"bodyTruncated": true` and `"totalBodyLines"` present in the definition object

**Validates:** `maxBodyLines` caps per-definition body output, truncation metadata is accurate.

**Note:** Replace `<some_known_long_def>` with a definition that has more than 5 lines of body.

---

### T28c: `serve` — search_definitions backward compatibility (default `includeBody: false`)

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"name":"<some_known_def>"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT --definitions
```

**Expected:**

- stdout: JSON-RPC response with definition results
- Definition objects do NOT contain a `"body"` field — same output as before the feature was added

**Validates:** Backward compatibility — omitting `includeBody` (or defaulting to `false`) produces the original response format.

---

### T28d: `serve` — search_definitions with `containsLine` + `includeBody: true`

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"file":"<known_file>","containsLine":<known_line>,"includeBody":true}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT --definitions
```

**Expected:**

- stdout: JSON-RPC response with definition results
- Result includes `"containingDefinitions"` array
- Each containing definition has a `"bodyStartLine"` (integer, 1-based) and `"body"` array (string array of source lines)

**Validates:** `includeBody` works together with `containsLine` mode, body is attached to containing definitions.

**Note:** Replace `<known_file>` and `<known_line>` with a file path and line number known to be inside a definition.

---

### T28e: `serve` — search_definitions with `maxTotalBodyLines` budget exhaustion

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"parent":"<class_with_many_methods>","includeBody":true,"maxTotalBodyLines":20}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT --definitions
```

**Expected:**

- stdout: JSON-RPC response with definition results
- First few definitions have `"body"` arrays with content
- Later definitions have `"bodyOmitted"` marker (body budget exhausted)
- Total body lines across all definitions ≤ 20

**Validates:** `maxTotalBodyLines` global budget is enforced, definitions beyond the budget get `bodyOmitted`, budget is reported accurately.

**Note:** Replace `<class_with_many_methods>` with a class/parent that has many method definitions in the indexed codebase.

---

### T28f: `serve` — search_definitions by attribute returns no duplicates

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"0.1.0"}}}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"attribute":"<attribute_name>","kind":"class"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT --definitions
```

**Expected:**

- stdout: JSON-RPC response with definition results
- No duplicate entries: each class appears at most once, even if it has the same attribute applied multiple times (e.g., `[ServiceProvider]` and `[ServiceProvider("config")]`)
- `totalResults` matches the count of unique definitions in the `definitions` array

**Validates:** Attribute index deduplication — a class with multiple attributes normalizing to the same name (e.g., `Attr` and `Attr("arg")`) is indexed only once per attribute name.

**Note:** Replace `<attribute_name>` with an attribute that some classes use multiple times with different arguments.

---

### T29: `serve` — MCP search_callers (requires --definitions)

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"tokenize","depth":2}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT --definitions
```

**Expected:**

- stdout: JSON-RPC response with call tree
- Result includes `callTree` array, `query` object (method, direction, depth), `summary` object (totalNodes, searchTimeMs)
- For Rust codebase: empty callTree (tree-sitter supports C#/TypeScript/SQL only)

**Validates:** search_callers handler end-to-end, call tree building, JSON output format.

**Note:** For C# codebases, use a method name that exists (e.g., `ExecuteQueryAsync`).

---

### T30: `serve` — MCP search_callers with class filter and direction=down

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"tokenize","class":"SomeClass","direction":"down","depth":2}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT --definitions
```

**Expected:**

- stdout: JSON-RPC response with callee tree
- `query.direction` = "down"
- `query.class` = "SomeClass" (class filter passed through)
- Result includes `callTree`, `summary`

**Validates:** class parameter works for direction=down (bug fix), callee tree building.

---

### T31: `serve` — search_callers finds callers through prefixed fields (C# only)

**Command (C# codebase with field naming like `m_orderProcessor` or `_userService`):**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"<MethodName>","class":"<ClassName>","depth":1}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext cs --definitions
```

**Expected:**

- `callTree` includes callers from files that reference the class only through a prefixed field (e.g., `m_className`, `_className`, `s_className`)
- Uses trigram index for substring matching in the `parent_file_ids` filter
- If trigram index is not built (e.g., fresh startup, never used `substring` search), callers through prefixed fields may be missed — this is expected (no crash, no regression)

**Validates:** Fix for field-prefix bug where `m_orderProcessor.SubmitAsync()` was missed because `m_orderprocessor` token ≠ `orderprocessor` token. Trigram substring matching in `collect_substring_file_ids()`.

---

### T32: `serve` — search_callers works with multi-extension `--ext` flag

**Command (server started with `--ext cs,csproj,xml,config`):**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"<MethodName>","depth":1}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext cs,csproj,xml,config --definitions
```

**Expected:**

- `callTree` is NOT empty (if the method exists and has callers)
- Files with `.cs` extension are NOT filtered out despite `--ext` containing multiple comma-separated extensions
- Previously this was broken: ext_filter compared `"cs"` against the entire string `"cs,csproj,xml,config"` → no match → all files filtered out

**Validates:** Fix for ext_filter comma-split bug. `build_caller_tree` and `build_callee_tree` now split ext_filter on commas before comparing.

---

### T33: `serve` — search_grep with `substring: true` (basic)

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"tokeniz","substring":true}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC response with search results
- Result content includes files containing tokens that have `tokeniz` as a substring (e.g., `tokenize`)
- Result includes `matchedTokens` field listing matched index tokens
- `summary.totalFiles` > 0

**Validates:** Substring search via trigram index, `matchedTokens` in response.

**Status:** ✅ Implemented (covered by `e2e_substring_search_full_pipeline` unit test)

---

### T34: `serve` — search_grep with `substring: true` + short query warning

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"fn","substring":true}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC response with search results
- Result includes a `"warning"` field about short substring queries (<4 chars)

**Validates:** Short query warning for substring search.

**Status:** ✅ Implemented (covered by `e2e_substring_search_short_query_warning` unit test)

---

### T35: `serve` — search_grep with `substring: true` + `showLines: true`

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"tokeniz","substring":true,"showLines":true,"maxResults":2}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC response with search results
- Each file object contains a `"lineContent"` array with compact grouped format
- Lines contain the matched substring

**Validates:** Substring search combined with `showLines`.

**Status:** ✅ Implemented (covered by `e2e_substring_search_with_show_lines` unit test)

---

### T36: `serve` — search_grep `substring: true` mutually exclusive with `regex`

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"test","substring":true,"regex":true}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC error response indicating `substring` and `regex` are mutually exclusive

**Validates:** Mutual exclusivity between substring and regex modes.

**Status:** ✅ Implemented (covered by `e2e_substring_mutually_exclusive_with_regex` unit test)

---

### T37: `serve` — search_grep `substring: true` mutually exclusive with `phrase`

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"pub fn","substring":true,"phrase":true}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC error response indicating `substring` and `phrase` are mutually exclusive

**Validates:** Mutual exclusivity between substring and phrase modes.

**Status:** ✅ Implemented (covered by `e2e_substring_mutually_exclusive_with_phrase` unit test)

---

### T37a: `serve` — search_grep defaults to substring mode (no explicit param)

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"tokenize"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC response with `searchMode` containing `"substring"` (not `"or"` or `"and"`)
- Results should include compound token matches (e.g., `"tokenize_basic"` if present)

**Validates:** `substring` defaults to `true` when no explicit `substring` parameter is passed. This ensures compound C# identifiers (e.g., `ICatalogQueryManager`, `m_catalogQueryManager`) are always found without the LLM needing to remember to pass `substring: true`.

**Status:** ✅ Implemented (covered by `test_substring_default_finds_compound_identifiers` unit test + T28 in e2e-test.ps1)

---

### T37b: `serve` — regex auto-disables substring (no error)

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":".*stale.*","regex":true}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- stdout: JSON-RPC response with search results (NOT an error)
- `searchMode` should NOT contain `"substring"` (regex is used instead)

**Validates:** When `regex: true` is passed without explicit `substring: false`, substring is auto-disabled (not an error). Only explicit `substring: true` + `regex: true` should error.

**Status:** ✅ Implemented (covered by `test_regex_auto_disables_substring` unit test + T29 in e2e-test.ps1)

---
### T37c: `serve` — search_grep substring AND-mode correctness (no false positives from multi-token match)

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"service,controller","substring":true,"mode":"and"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- Results only include files that contain tokens matching BOTH `service` AND `controller` as substrings
- A file containing only `userservice`, `servicehelper`, `servicemanager` (3 tokens matching `service`) but NO token matching `controller` must NOT appear in results
- Previously, `terms_matched` was incremented per matching token (not per search term), so a file with 3 `service`-matching tokens would get `terms_matched=3`, falsely passing the AND filter `terms_matched >= 2`

**Validates:** Fix for AND-mode correctness bug in substring search. `terms_matched` now counts distinct search terms, not matching tokens.

**Status:** ✅ Implemented (covered by `test_substring_and_mode_no_false_positive_from_multi_token_match` unit test)

---


### T38: `serve` — search_reindex rebuilds trigram index

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_reindex","arguments":{}}}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"tokeniz","substring":true}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- Reindex response: success
- Subsequent substring search: works correctly, `totalFiles` > 0

**Validates:** Reindex flow rebuilds trigram index alongside content index.

**Status:** ✅ Implemented (covered by `e2e_reindex_rebuilds_trigram` unit test)

---

### T39: `serve` — MCP initialize includes `instructions` field

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- JSON-RPC response `result` contains `instructions` field (string)
- `instructions` mentions `search_fast`, `search_find`, `substring`, `search_callers`, `class`, `includeBody`, `countOnly`
- Provides LLM-readable best practices for tool selection

**Validates:** MCP server-level instructions for LLM tool selection guidance.

**Status:** ✅ Implemented (covered by `test_initialize_includes_instructions` unit test)

---

### T40: `serve` — MCP search_help returns best practices

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_help","arguments":{}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext $TEST_EXT
```

**Expected:**

- JSON response with `bestPractices` array (6 items covering file lookup, substring, call chain, class param, includeBody, countOnly)
- `performanceTiers` object with instant/fast/quick/slow tiers
- `toolPriority` array with recommended tool order

**Validates:** On-demand best practices guide for LLMs.

---

### T41: `grep` — Non-code file search (csproj, xml, config)

**Setup:**

Create a temporary directory with a `.csproj` file:

```powershell
$tmp = New-Item -ItemType Directory -Path "$env:TEMP\search_noncode_test_$(Get-Random)"
@'
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
    <PackageReference Include="Serilog" Version="3.1.1" />
  </ItemGroup>
</Project>
'@ | Set-Content "$tmp\TestProject.csproj"
cargo run -- content-index -d $tmp -e csproj
```

**Command:**

```powershell
cargo run -- grep "Newtonsoft.Json" -d $tmp -e csproj
```

**Expected:**

- Exit code: 0
- stdout: `TestProject.csproj` listed as a match
- File contains the NuGet package reference

**Cleanup:**

```powershell
Remove-Item -Recurse -Force $tmp
```

**Validates:** `search_grep` works with non-code file extensions like `.csproj`. Users can search NuGet dependencies, XML configurations, and other non-code files by including the appropriate extension in `--ext`.

---

### T41a: `serve` — MCP search_grep with ext='csproj' override

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"Newtonsoft.Json","ext":"csproj"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $tmp --ext csproj
```

**Expected:**

- JSON-RPC response with matching file(s) containing `Newtonsoft.Json`
- `ext` parameter override filters to `.csproj` files only

**Validates:** MCP `search_grep` `ext` parameter works with non-code extensions.

---

### T41b: `tips` / `search_help` — Non-code file tip present

**Command (CLI):**

```powershell
cargo run -- tips
```

**Expected:**

- Output contains tip about searching non-code file types (XML, csproj, config)
- Mentions `ext='csproj'` or similar example

**Validates:** The new tip for non-code file search is visible in CLI output and MCP `search_help`.

---

### T42: `tips` / `search_help` — Strategy recipes present

**Command (CLI):**

```powershell
cargo run -- tips
```

**Expected:**

- Output contains "STRATEGY RECIPES" section
- Contains "Architecture Exploration" recipe with steps and anti-patterns
- Contains "Call Chain Investigation" recipe
- Contains "Stack Trace / Bug Investigation" recipe

**Command (MCP):**

```powershell
$input = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_help","arguments":{}}}'
) -join "`n"
echo $input | cargo run -- serve -d $TEST_DIR -e $TEST_EXT
```

**Expected:**

- JSON response contains `strategyRecipes` array with 3 entries
- Each recipe has `name`, `when`, `steps`, `antiPatterns` fields

**Validates:** Strategy recipes are exposed in both CLI and MCP outputs.

---

### T42b: `tips` / `search_help` — Query budget and multi-term tips present

**Command (CLI):**

```powershell
cargo run -- tips
```

**Expected:**

- Output contains tip about "Query budget: aim for 3 or fewer search calls"
- Output contains tip about "Multi-term name in search_definitions"
- Multi-term tip mentions comma-separated example: `UserService,IUserService,UserController`

**Validates:** New efficiency guidance tips are visible in CLI output and MCP `search_help`.

---


## TypeScript Support Tests

### T44: `def-index` — Build TypeScript definition index

**Command:**

```powershell
cargo run -- def-index -d $TEST_DIR -e ts
```

**Expected:**

- Exit code: 0
- stderr: `[def-index] Found N files to parse`
- stderr: `[def-index] Parsed N files in X.Xs, extracted M definitions`
- A `.didx` file created
- Definitions include TypeScript-specific kinds: `function`, `class`, `interface`, `enum`, `typeAlias`, `variable`

**Validates:** Tree-sitter TypeScript parsing, definition extraction for `.ts` files.

---

### T45: `def-index` — Build TypeScript + TSX definition index

**Command:**

```powershell
cargo run -- def-index -d $TEST_DIR -e ts,tsx
```

**Expected:**

- Exit code: 0
- stderr: `[def-index] Found N files to parse` (N includes both `.ts` and `.tsx` files)
- stderr: `[def-index] Parsed N files in X.Xs, extracted M definitions`
- Definitions extracted from both `.ts` and `.tsx` files

**Validates:** Mixed `.ts` + `.tsx` extension handling in definition indexing. TSX files are parsed using the TSX grammar.

---

### T46: `serve` — MCP search_definitions finds TypeScript functions

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"kind":"function"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext ts --definitions
```

**Expected:**

- stdout: JSON-RPC response with definition results
- Results contain TypeScript function declarations with `kind: "function"`
- Each definition includes `name`, `file`, `lines`, `signature`

**Validates:** `search_definitions` with `kind` filter works for TypeScript-specific definition kinds.

**Note:** Requires a TypeScript project with function declarations.

---

### T47: `serve` — MCP search_definitions finds TypeScript class by name

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"name":"UserService"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext ts --definitions
```

**Expected:**

- stdout: JSON-RPC response with definition results matching `UserService`
- Result includes class definition with correct file path and line range

**Validates:** Name-based search works for TypeScript definitions.

**Note:** Replace `UserService` with a class name that exists in the TypeScript project.

---

### T48: `serve` — MCP search_definitions finds decorated TypeScript classes

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"attribute":"injectable"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext ts --definitions
```

**Expected:**

- stdout: JSON-RPC response with definition results
- Results contain TypeScript classes decorated with `@Injectable()` or similar decorators
- Decorator names are stored as attributes (lowercased, without `@` prefix)

**Validates:** TypeScript decorator extraction and attribute-based search.

**Note:** Replace `injectable` with a decorator name that exists in the TypeScript project.

---

### T49: `def-index` — Mixed C# + TypeScript definition index

**Command:**

```powershell
cargo run -- def-index -d $TEST_DIR -e cs,ts
```

**Expected:**

- Exit code: 0
- stderr: `[def-index] Found N files to parse` (N includes both `.cs` and `.ts` files)
- stderr: `[def-index] Parsed N files in X.Xs, extracted M definitions`
- Both C# definitions (classes, methods, etc.) and TypeScript definitions (functions, type aliases, etc.) are present in the index

**Validates:** Mixed-language definition indexing. C# files use the C# parser, TypeScript files use the TypeScript parser, and both coexist in the same `.didx` index.

---

### T50: `serve` — Incremental TypeScript definition update via watcher

**Scenario:** Start the MCP server with `--watch --definitions` for a TypeScript project. Modify a `.ts` file (add or rename a function). The watcher should detect the change and re-parse the file, updating definitions in-place.

**Command:**

```powershell
# Start server in background
$server = Start-Process -PassThru -NoNewWindow cargo -ArgumentList "run -- serve --dir $TEST_DIR --ext ts --watch --definitions"

# Wait for server to initialize
Start-Sleep -Seconds 3

# Modify a .ts file (add a new function)
Add-Content "$TEST_DIR\some_file.ts" "`nexport function newTestFunction(): void { }"

# Wait for watcher debounce
Start-Sleep -Seconds 2

# Query for the new function
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"name":"newTestFunction"}}}'
) -join "`n"
echo $msgs | & $server.Path
```

**Expected:**

- After file modification, stderr shows watcher detecting the change
- `search_definitions` finds `newTestFunction` with correct file and line info

**Validates:** Incremental definition update for TypeScript files via the file watcher.

**Note:** This is a manual test requiring a running server. Clean up the added function after testing.

---

### T51: `serve` — TypeScript-specific definition kinds (typeAlias, variable)

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"kind":"typeAlias"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext ts --definitions
```

**Expected:**

- stdout: JSON-RPC response with definitions of kind `typeAlias`
- Results contain TypeScript `type` declarations (e.g., `type Props = { ... }`)

**Command (variable kind):**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_definitions","arguments":{"kind":"variable"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext ts --definitions
```

**Expected:**

- stdout: JSON-RPC response with definitions of kind `variable`
- Results contain exported `const`/`let`/`var` declarations

**Validates:** TypeScript-specific definition kinds (`typeAlias`, `variable`) are correctly extracted and searchable.

**Note:** Requires a TypeScript project with type aliases and exported variables.

---

### T52: `serve` — Response truncation for `search_definitions` broad queries

**Scenario:** When `search_definitions` returns a large result set (e.g., broad `kind: "property"`
query on a large codebase), the response must be truncated to stay within the `--max-response-kb`
budget. Unlike `search_grep` (which uses Phase 1-4 with its `files` array structure),
`search_definitions` uses a `definitions` array — truncation Phase 5 (generic array fallback)
handles this. The `summary` must include truncation metadata with a definitions-specific hint.

**Command (broad query expected to exceed 16KB):**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_definitions","arguments":{"kind":"property","maxResults":500}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext cs,ts,tsx --definitions --metrics 2>$null
```

**Expected (if > 16KB):**

- `summary.responseTruncated` = `true`
- `summary.truncationReason` contains `"truncated 'definitions' array"`
- `summary.returned` matches actual `definitions` array length (not the pre-truncation count)
- `summary.totalResults` reflects the full match count (before both `maxResults` and Phase 5 truncation)
- `summary.hint` mentions `"name, kind, file, or parent filters"` (NOT `"countOnly"`)
- `summary.originalResponseBytes` > response budget

**Negative test — narrow query stays under budget:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_definitions","arguments":{"name":"truncate_large_response","kind":"method"}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir . --ext rs --definitions --metrics 2>$null
```

**Expected:**

- `summary.responseTruncated` is absent
- `definitions` array contains the full result set

**Validates:** Phase 5 generic array truncation for non-grep response formats, definitions-specific hint, `returned` count accuracy after truncation.

**Note:** Requires a large enough codebase with 500+ properties to trigger truncation. If the test codebase is small, increase `maxResults` or use `--max-response-kb 4` to lower the budget.

---

## TypeScript Callers Tests

### T53: `serve` — search_callers finds TypeScript class method callers

**Command (TypeScript codebase):**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"<MethodName>","class":"<ClassName>","depth":2}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext ts --definitions
```

**Expected:**

- stdout: JSON-RPC response with call tree
- `callTree` includes callers from TypeScript files where `this.service.method()` pattern is used
- Caller entries have correct `class` (receiver type resolved from field type map)
- `query.method` matches the requested method name

**Validates:** TypeScript call-site extraction for class method calls via `this.field.method()` pattern. Receiver type is resolved through the field type map built from class fields and constructor parameter properties.

**Note:** Replace `<MethodName>` and `<ClassName>` with a method/class that exists in the TypeScript project.

---

### T54: `serve` — search_callers finds TypeScript standalone function calls

**Command (TypeScript codebase with standalone functions):**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"<functionName>","depth":1}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext ts --definitions
```

**Expected:**

- stdout: JSON-RPC response with call tree
- `callTree` includes callers that invoke the standalone function
- Callers may include both `DefinitionKind::Function` and `DefinitionKind::Method` entries
- Standalone function calls have no receiver type (bare `functionName()` calls)

**Validates:** TypeScript standalone function call-site extraction. Functions are recognized as valid "containing method" entries in the caller tree (via `DefinitionKind::Function` support in `find_containing_method`).

**Note:** Standalone function calls without a receiver may be ambiguous — the callers tool finds them by method name grep, not by import resolution.

---

### T55: `serve` — search_callers with `ext` parameter filters by language

**Command (mixed C#/TypeScript codebase):**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"<MethodName>","ext":"ts","depth":1}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext cs,ts --definitions
```

**Expected:**

- stdout: JSON-RPC response with call tree
- All results are from `.ts` files only (no `.cs` files)
- `ext` parameter filters both the grep search and the definition lookups

**Validates:** `ext` parameter on `search_callers` can filter results to a specific language in a mixed-language project.

**Note:** Server must be started with `--ext cs,ts` to index both languages. The `ext` parameter in the tool call narrows results to TypeScript only.

---

### T56: `serve` — search_callers finds callers in TypeScript arrow function class properties

**Command (TypeScript codebase with arrow function class properties):**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"<MethodName>","depth":1}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext ts --definitions
```

**Expected:**

- stdout: JSON-RPC response with call tree
- `callTree` includes callers from arrow function class properties (e.g., `processItem = (item: Item): void => { this.validate(item); }`)
- The arrow function property is treated as a method for call-site extraction purposes

**Validates:** Arrow function class properties (`public_field_definition` with `arrow_function` initializer) are recognized as call-site sources. Call sites within arrow function bodies are extracted correctly.

**Note:** Replace `<MethodName>` with a method that is called from within an arrow function class property.

---

### T57: `serve` — search_callers tracks TypeScript `new` expression constructor calls

**Command (TypeScript codebase with constructor calls):**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"<ClassName>","depth":1}}}'
) -join "`n"
echo $msgs | cargo run -- serve --dir $TEST_DIR --ext ts --definitions
```

**Expected:**

- stdout: JSON-RPC response with call tree
- `callTree` includes callers that use `new ClassName(...)` expressions
- Constructor calls have `receiver_type` matching the class name

**Validates:** TypeScript `new_expression` nodes are extracted as call sites. `new UserService(logger)` is tracked as a call to `UserService` with receiver type `UserService`.

**Note:** Replace `<ClassName>` with a class that is instantiated via `new` in the TypeScript project.

### T58: `serve` — search_callers resolves Angular `inject()` field types

**Command (Angular/TypeScript codebase with `inject()` usage):**

```powershell
@(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"<MethodName>","depth":1}}}'
) -join "`n"
```

Replace `<MethodName>` with a method called on an `inject()`-resolved field (e.g., `dispatch` if `this.store = inject(Store)` and `this.store.dispatch()` is called).

**Expected:**

- stdout: JSON-RPC response with call tree
- `callTree` includes callers where the receiver type is resolved from `inject(ClassName)` patterns
- Two patterns are supported:
  - **Field initializer**: `private store = inject(Store);` → `this.store.dispatch()` resolves receiver to `Store`
  - **Constructor assignment**: `this.router = inject(Router);` → `this.router.navigate()` resolves receiver to `Router`
- Generic type arguments are stripped: `inject(Store<AppState>)` → receiver type is `Store`

**Validates:** Angular `inject()` function support for field type resolution. The TypeScript parser extracts `inject(ClassName)` calls from both field initializers and constructor assignments, adding them to the per-class field type map used by `resolve_ts_receiver_type()`.

**Note:** Replace `<MethodName>` with a method that is called on an `inject()`-resolved field in the Angular/TypeScript project.

---

### T59: `serve` — search_callers ambiguity warning truncated for common methods

**Command (codebase with many classes implementing the same method, e.g., Angular `ngOnInit`):**

```powershell
@(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"ngOnInit"}}}'
) -join "`n"
```

**Expected:**

- stdout: JSON-RPC response with call tree and a `"warning"` field
- Warning mentions total number of classes (e.g., "found in 1899 classes")
- Warning lists at most 10 class names followed by "…" (truncated)
- Warning does NOT list all classes — total warning length stays under ~500 bytes regardless of how many classes contain the method
- Warning advises using the `class` parameter to scope the search

**Validates:** Ambiguity warning is truncated when a method name (without `class` filter) matches many classes. Previously, the warning listed all class names, producing ~56KB responses (~14K tokens) for common methods like `ngOnInit`.

**Status:** ✅ Implemented (covered by `test_search_callers_ambiguity_warning_truncated` unit test)

---

## Automation Script

Save as `e2e-test.ps1` and run from workspace root:

```powershell
#!/usr/bin/env pwsh
param(
    [string]$TestDir = ".",
    [string]$TestExt = "rs",
    [string]$Binary = "cargo run --"
)

$ErrorActionPreference = "Stop"
$passed = 0
$failed = 0
$total = 0

function Run-Test {
    param([string]$Name, [string]$Command, [int]$ExpectedExit = 0, [string]$StderrContains = "", [string]$StdoutContains = "")

    $script:total++
    Write-Host -NoNewline "  $Name ... "

    $result = Invoke-Expression "$Command 2>&1"
    $exitCode = $LASTEXITCODE

    $output = $result -join "`n"

    if ($exitCode -ne $ExpectedExit) {
        Write-Host "FAILED (exit=$exitCode, expected=$ExpectedExit)" -ForegroundColor Red
        $script:failed++
        return
    }

    if ($StdoutContains -and -not ($output -match [regex]::Escape($StdoutContains))) {
        Write-Host "FAILED (output missing: $StdoutContains)" -ForegroundColor Red
        $script:failed++
        return
    }

    Write-Host "OK" -ForegroundColor Green
    $script:passed++
}

Write-Host "`n=== E2E Tests (dir=$TestDir, ext=$TestExt) ===`n"

# Build first
Write-Host "Building..."
& cargo build 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "Build failed!" -ForegroundColor Red; exit 1 }

# T01-T05: find
Run-Test "T01 find-filename"       "$Binary find main -d $TestDir -e $TestExt"
Run-Test "T02 find-contents"       "$Binary find `"fn main`" -d $TestDir -e $TestExt --contents"
Run-Test "T04 find-case-insensitive" "$Binary find CONTENTINDEX -d $TestDir -e $TestExt --contents -i"
Run-Test "T05 find-count"          "$Binary find fn -d $TestDir -e $TestExt --contents -c"

# T06-T09: index + fast
Run-Test "T06 index-build"         "$Binary index -d $TestDir"
Run-Test "T07 fast-search"         "$Binary fast main -d $TestDir -e $TestExt"

# T10: content-index
Run-Test "T10 content-index"       "$Binary content-index -d $TestDir -e $TestExt"

# T11-T18: grep
Run-Test "T11 grep-single"         "$Binary grep tokenize -d $TestDir -e $TestExt"
Run-Test "T12 grep-multi-or"       "$Binary grep `"tokenize,posting`" -d $TestDir -e $TestExt"
Run-Test "T13 grep-multi-and"      "$Binary grep `"tokenize,posting`" -d $TestDir -e $TestExt --all"
Run-Test "T14 grep-regex"          "$Binary grep `".*stale.*`" -d $TestDir -e $TestExt --regex"
Run-Test "T15 grep-phrase"         "$Binary grep `"pub fn`" -d $TestDir -e $TestExt --phrase"
Run-Test "T16 grep-context"        "$Binary grep is_stale -d $TestDir -e $TestExt --show-lines -C 2 --max-results 2"
Run-Test "T17 grep-exclude"        "$Binary grep ContentIndex -d $TestDir -e $TestExt --exclude-dir bench"
Run-Test "T18 grep-count"          "$Binary grep fn -d $TestDir -e $TestExt -c"
Run-Test "T24 grep-before-after"   "$Binary grep is_stale -d $TestDir -e $TestExt --show-lines -B 1 -A 3"

# T19: info
Run-Test "T19 info"                "$Binary info"

# T20: def-index
Run-Test "T20 def-index"           "$Binary def-index -d $TestDir -e $TestExt"

# T21-T23: error handling
Run-Test "T21 invalid-regex"       "$Binary grep `"[invalid`" -d $TestDir -e $TestExt --regex" -ExpectedExit 1
Run-Test "T22 nonexistent-dir"     "$Binary find test -d /nonexistent/path/xyz" -ExpectedExit 1

# T20b: def-index with TypeScript (T49 — mixed C#/TS)
# Only runs if TestExt includes ts/tsx or if we detect .ts files
Run-Test "T49 def-index-ts"        "$Binary def-index -d $TestDir -e ts"

# T25-T52: serve (MCP)
# Note: MCP tests require piping JSON-RPC to stdin, which is hard to automate in simple PowerShell.
# These are manual verification tests — run them individually per the test plan.
# Includes: T25-T30 (grep/find MCP), T42 (grep truncation), T44-T51 (TypeScript definitions),
#           T52 (definitions truncation Phase 5).
Write-Host "  T25-T52: MCP serve tests — run manually (see e2e-test-plan.md)"

Write-Host "`n=== Results: $passed passed, $failed failed, $total total ===`n"
if ($failed -gt 0) { exit 1 }
```

**Usage:**

```powershell
# Default (current workspace, .rs files)
./e2e-test.ps1

# Custom directory
./e2e-test.ps1 -TestDir "C:\Projects\MyApp" -TestExt "cs"

# With release binary
./e2e-test.ps1 -Binary "./target/release/search"
```

---

## When to Run

- ✅ After every major refactoring or structural change
- ✅ After dependency upgrades (`cargo update`)
- ✅ Before creating a PR
- ✅ After merging a large PR
- ✅ When switching Rust toolchain versions


### T30: `serve` — MCP search_grep with subdirectory `dir` parameter

**Scenario:** When the MCP server is started with `--dir C:\Repos\Shared`, a `search_grep` call
with `dir` set to a subdirectory (e.g., `C:\Repos\Shared\Sql\CloudBI`) should succeed and
return only files within that subdirectory. Previously this returned an error.

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"main","dir":"src/mcp"}}}'
) -join "`n"
$msgs | cargo run -- serve -d . -e rs 2>$null
```

**Expected:**

- No error about "For other directories, start another server instance"
- Results contain only files whose path includes `src/mcp`
- `summary.totalFiles` ≥ 1

**Negative test — directory outside server dir:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"main","dir":"Z:\\other\\path"}}}'
) -join "`n"
$msgs | cargo run -- serve -d . -e rs 2>$null
```

**Expected:**

- Response contains error: "Server started with --dir"
- Tool result `isError: true`


---

### T42: `serve` — Response size truncation for broad queries

**Scenario:** When a search query returns massive results (e.g., short substring query matching
thousands of files), the MCP server automatically truncates the JSON response to stay within
~32KB to prevent filling the LLM context window. Truncation is progressive:
1. Cap `lines` arrays per file to 10 entries
2. Remove `lineContent` blocks
3. Cap `matchedTokens` to 20 entries
4. Remove `lines` arrays entirely
5. Reduce file count

**Command:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"fn","substring":true}}}'
) -join "`n"
$msgs | cargo run -- serve -d . -e rs --metrics 2>$null
```

**Expected:**

- `summary.responseTruncated` = `true`
- `summary.truncationReason` contains truncation phases applied
- `summary.originalResponseBytes` > 32768
- `summary.responseBytes` ≤ ~33000 (under budget with small metadata overhead)
- `summary.hint` contains advice to use `countOnly` or narrow filters
- `summary.totalFiles` and `summary.totalOccurrences` reflect the FULL result set (not truncated)
- The `files` array is reduced from 50 to a smaller number

**Negative test — small query is NOT truncated:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"truncate_large_response"}}}'
) -join "`n"
$msgs | cargo run -- serve -d . -e rs --metrics 2>$null
```

**Expected:**

- `summary.responseTruncated` is absent (response under budget)
- `summary.responseBytes` < 32768

**Validates:** Progressive response truncation, LLM context budget protection, summary metadata accuracy.


---

### T43: `serve` — search_find directory validation (security)

**Scenario:** The `search_find` tool now validates the `dir` parameter against `server_dir`,
matching the same security behavior as `search_grep`. Previously, `search_find` accepted any
directory path, allowing filesystem enumeration outside the server's configured scope.

**Test — directory outside `server_dir` is rejected:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_find","arguments":{"pattern":"*","dir":"C:\\Windows"}}}'
) -join "`n"
$msgs | cargo run -- serve -d . -e rs 2>$null
```

**Expected:**

- Response contains error indicating directory is outside allowed scope
- Tool result `isError: true`
- Error message references `--dir` / `server_dir`

**Test — subdirectory of `server_dir` is accepted:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_find","arguments":{"pattern":"*.rs","dir":"src/mcp"}}}'
) -join "`n"
$msgs | cargo run -- serve -d . -e rs 2>$null
```

**Expected:**

- No error
- Results contain file paths within `src/mcp`
- Normal `search_find` output with match count

**Test — no `dir` parameter uses `server_dir` as default:**

```powershell
$msgs = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_find","arguments":{"pattern":"*.rs"}}}'
) -join "`n"
$msgs | cargo run -- serve -d . -e rs 2>$null
```

**Expected:**

- No error
- Results returned from the server's root directory
- Normal `search_find` output

**Validates:** `search_find` directory validation parity with `search_grep`, preventing filesystem enumeration outside allowed scope.

**Status:** ✅ Implemented (covered by `test_validate_search_dir_subdirectory` and `test_validate_search_dir_outside_rejects` unit tests)


---

### T-LOSSY: Non-UTF8 file indexing (lossy UTF-8 conversion)

**Background:** Files with Windows-1252 encoded characters (e.g., smart quotes `'` = byte `0x92` in comments) were previously silently skipped during definition indexing because `std::fs::read_to_string()` requires valid UTF-8. This test verifies that such files are now indexed via lossy UTF-8 conversion.

**Setup:**

```powershell
# Create a temp directory with a .cs file containing a non-UTF8 byte
$testDir = "$env:TEMP\search_e2e_lossy"
New-Item -ItemType Directory -Force -Path $testDir | Out-Null
$bytes = [System.Text.Encoding]::UTF8.GetBytes(@"
using System;
namespace TestApp
{
    // Comment: you
"@)
$bytes += [byte]0x92  # Windows-1252 right single quote
$bytes += [System.Text.Encoding]::UTF8.GetBytes(@"re a dev
    public class DataProcessor
    {
        public void Process() { }
    }
}
"@)
[System.IO.File]::WriteAllBytes("$testDir\Program.cs", $bytes)
```

**Command:**

```powershell
cargo run -- def-index --dir $testDir --ext cs
```

**Expected:**

- Exit code: 0
- stderr contains: `WARNING: file contains non-UTF8 bytes (lossy conversion applied)`
- stderr contains: `1 lossy-utf8 files`
- stderr contains: `extracted` with a non-zero definition count

**Verify definitions are indexed:**

```powershell
# Start MCP server and query
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' | cargo run -- serve --dir $testDir --ext cs --definitions 2>$null
```

Then send `search_definitions` with `file: "Program.cs"` — should return `DataProcessor` class and `Process` method.

**Cleanup:**

```powershell
Remove-Item -Recurse -Force $testDir
```


---

### T-AUDIT: Definition index audit mode

**Background:** The `search_definitions` tool supports an `audit` parameter that returns index coverage statistics — how many files have definitions, how many are empty, and which suspicious files (large but 0 definitions) may have parsing issues.

**Prerequisites:** Server running with `--definitions` flag

**Command (MCP JSON-RPC):**

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_definitions","arguments":{"audit":true}}}
```

**Expected response structure:**

```json
{
  "audit": {
    "totalFiles": "<number>",
    "filesWithDefinitions": "<number>",
    "filesWithoutDefinitions": "<number>",
    "readErrors": "<number>",
    "lossyUtf8Files": "<number>",
    "suspiciousFiles": "<number>",
    "suspiciousThresholdBytes": 500
  },
  "suspiciousFiles": ["<array of {file, bytes}>"]
}
```

**Assertions:**
- `audit.totalFiles` > 0
- `audit.filesWithDefinitions` > 0
- `audit.filesWithDefinitions` + `audit.filesWithoutDefinitions` ≤ `audit.totalFiles`
- `audit.readErrors` ≥ 0
- `suspiciousFiles` is an array
- Each entry in `suspiciousFiles` has `file` (string) and `bytes` (number > threshold)

**With custom threshold:**

```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_definitions","arguments":{"audit":true,"auditMinBytes":10000}}}
```

Should return fewer suspicious files (only those >10KB with 0 definitions).


---

### T-DEF-AUDIT: Definition index audit CLI command

**Background:** The `search def-audit` CLI subcommand loads a previously built `.didx` file from disk and reports index coverage: how many files have definitions, how many are empty, and which suspicious files (large but 0 definitions) may have parsing issues. This does NOT rebuild the index.

**Prerequisites:** A definition index must already be built via `search def-index`.

**Command:**

```powershell
# Build first (if not already built)
search def-index --dir $TEST_DIR --ext rs

# Audit (instant — loads from disk)
search def-audit --dir $TEST_DIR --ext rs
```

**Expected:**

- Exit code: 0
- stderr contains `[def-audit] Index:` with total files count
- stderr contains `with definitions` count > 0
- stderr contains `without definitions` count ≥ 0
- stderr contains `definitions,` followed by `read errors` and `lossy-UTF8 files`

**With custom threshold:**

```powershell
search def-audit --dir $TEST_DIR --ext rs --min-bytes 10000
```

- Should show fewer suspicious files (only those >10KB with 0 definitions)

**When no index exists:**

```powershell
search def-audit --dir C:\nonexistent --ext cs
```

- stderr contains `No definition index found`
- Exit code: 0


---

### T60: `def-index` — Extension filtering (no unnecessary parsers)

**Purpose:** Verify that definition index only parses files matching requested extensions, and doesn't load TypeScript grammars for C#-only projects.

**Command (C# only):**

```powershell
cargo run -- def-index -d $TEST_DIR -e cs
```

**Expected:**

- Exit code: 0
- stderr: `[def-index] Found N files to parse` — only `.cs` files counted
- No TypeScript grammar loading errors
- Only C# definitions extracted

**Command (C# + TypeScript):**

```powershell
cargo run -- def-index -d $TEST_DIR -e cs,ts,tsx
```

**Expected:**

- Exit code: 0
- Both C# and TypeScript definitions extracted
- TS/TSX parsers created lazily only in threads that encounter TS/TSX files

**Validates:** Extension-based parser filtering prevents unnecessary grammar loading for single-language projects. Fixes performance regression where TypeScript parsers were eagerly loaded for C#-only repositories.

---

### T-LZ4: LZ4 index compression and backward compatibility

**Background:** All index files (.idx, .cidx, .didx) are now saved with LZ4 frame compression, prefixed by magic bytes `LZ4S`. The loader auto-detects compressed vs legacy uncompressed formats for backward compatibility.

**Test — compressed index roundtrip:**

```powershell
# Build a content index (will be LZ4-compressed)
cargo run -- content-index -d $TEST_DIR -e $TEST_EXT

# Verify the index file starts with LZ4 magic bytes
$idxDir = "$env:LOCALAPPDATA\search-index"
$cidxFile = Get-ChildItem $idxDir -Filter *.cidx | Select-Object -First 1
$bytes = [System.IO.File]::ReadAllBytes($cidxFile.FullName)
$magic = [System.Text.Encoding]::ASCII.GetString($bytes[0..3])
if ($magic -ne "LZ4S") { throw "Expected LZ4S magic, got: $magic" }

# Verify grep still works (index loads correctly)
cargo run -- grep "fn" -d $TEST_DIR -e $TEST_EXT
```

**Expected:**
- Index file starts with `LZ4S` magic bytes
- stderr shows compression ratio log: `Saved X.X MB → Y.Y MB (Z.Z× compression)`
- grep returns results (index deserializes correctly after compression)

**Test — backward compatibility with legacy uncompressed index:**

```powershell
# Create a legacy uncompressed index manually (for testing)
# This is covered by unit test `test_load_compressed_legacy_uncompressed`
# which writes raw bincode and verifies load_compressed can read it
```

**Expected:**
- `load_compressed` reads both LZ4-compressed and legacy uncompressed files
- No data loss or deserialization errors

**Validates:** LZ4 compression, magic byte detection, backward compatibility, compression ratio logging.

**Status:** ✅ Covered by unit tests: `test_save_load_compressed_roundtrip`, `test_load_compressed_legacy_uncompressed`, `test_load_compressed_missing_file_returns_none`, `test_compressed_file_smaller_than_uncompressed`
