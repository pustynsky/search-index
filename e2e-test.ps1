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
    param([string]$Name, [string]$Command, [int]$ExpectedExit = 0)

    $script:total++
    Write-Host -NoNewline "  $Name ... "

    $ErrorActionPreference = "Continue"
    $result = Invoke-Expression "$Command 2>&1" | Out-String
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = "Stop"

    if ($exitCode -ne $ExpectedExit) {
        Write-Host "FAILED (exit=$exitCode, expected=$ExpectedExit)" -ForegroundColor Red
        $script:failed++
        return
    }

    Write-Host "OK" -ForegroundColor Green
    $script:passed++
}

Write-Host "`n=== E2E Tests (dir=$TestDir, ext=$TestExt) ===`n"

# Build first
Write-Host "Building..."
$ErrorActionPreference = "Continue"
& cargo build 2>&1 | Out-Null
$ErrorActionPreference = "Stop"
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
Run-Test "T15b grep-phrase-punct"  "$Binary grep `"pub(crate)`" -d $TestDir -e $TestExt --phrase"
Run-Test "T16 grep-context"        "$Binary grep is_stale -d $TestDir -e $TestExt --show-lines -C 2 --max-results 2"
Run-Test "T17 grep-exclude"        "$Binary grep ContentIndex -d $TestDir -e $TestExt --exclude-dir bench"
Run-Test "T18 grep-count"          "$Binary grep fn -d $TestDir -e $TestExt -c"
Run-Test "T24 grep-before-after"   "$Binary grep is_stale -d $TestDir -e $TestExt --show-lines -B 1 -A 3"

# T61-T64: grep substring (default) and --exact
Run-Test "T61 grep-substring-default" "$Binary grep contentindex -d $TestDir -e $TestExt"
Run-Test "T62 grep-substring-and"     "$Binary grep `"contentindex,tokenize`" -d $TestDir -e $TestExt --all"
Run-Test "T63 grep-exact"             "$Binary grep contentindex -d $TestDir -e $TestExt --exact"
Run-Test "T64 grep-regex-no-substr"   "$Binary grep `".*stale.*`" -d $TestDir -e $TestExt --regex"

# T19: info
Run-Test "T19 info"                "$Binary info"

# T20: def-index
Run-Test "T20 def-index"           "$Binary def-index -d $TestDir -e $TestExt"

# T49: def-index with TypeScript
Run-Test "T49 def-index-ts"        "$Binary def-index -d $TestDir -e ts"

# T21-T23: error handling
Run-Test "T21 invalid-regex"       "$Binary grep `"[invalid`" -d $TestDir -e $TestExt --regex" -ExpectedExit 1
Run-Test "T22 nonexistent-dir"     "$Binary find test -d /nonexistent/path/xyz" -ExpectedExit 1

# T42/T42b: tips — strategy recipes and query budget
Run-Test "T42 tips-strategy-recipes" "$Binary tips | Select-String 'STRATEGY RECIPES'"
Run-Test "T42b tips-query-budget"    "$Binary tips | Select-String 'Query budget'"

# T25-T52: serve (MCP)
Write-Host "  T25-T52: MCP serve tests - run manually (see e2e-test-plan.md)"

# T53-T58: TypeScript callers (MCP)
Write-Host "  T53-T58: TypeScript callers MCP tests - run manually (see e2e-test-plan.md)"

Write-Host "`n=== Results: $passed passed, $failed failed, $total total ===`n"
if ($failed -gt 0) { exit 1 }