# E2E Test Plan — Search Engine

## Overview

This document defines end-to-end tests for the `search` binary. These tests exercise
real CLI commands against a real directory to verify the full pipeline: indexing, searching,
output format, and all feature flags.

**Run these tests after every major refactoring, before merging PRs, and after dependency upgrades.**

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

**Note:** For `.rs` files, 0 definitions is expected (parser supports C#/SQL only).
For C# projects, expect hundreds/thousands of definitions.

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

- stdout: JSON-RPC response with 7 tools: `search_grep`, `search_find`, `search_fast`, `search_info`, `search_reindex`, `search_definitions`, `search_callers`
- Each tool has `name`, `description`, `inputSchema`

**Validates:** Tool discovery, tool schema generation.

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
- For Rust codebase: 0 results (tree-sitter supports C#/SQL only)
- For C# codebase: results with `name`, `kind`, `file`, `lines`

**Validates:** search_definitions handler, definition index loading, AST-based search.

**Note:** Requires `--definitions` flag. For `.rs` files, 0 results is expected.

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
- For Rust codebase: empty callTree (tree-sitter supports C#/SQL only)

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

# T25-T30: serve (MCP)
# Note: MCP tests require piping JSON-RPC to stdin, which is hard to automate in simple PowerShell.
# These are manual verification tests — run them individually per the test plan.
Write-Host "  T25-T30: MCP serve tests — run manually (see e2e-test-plan.md)"

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
