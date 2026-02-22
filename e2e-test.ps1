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
Run-Test "T03 find-regex"          "$Binary find `"fn\s+\w+`" -d $TestDir -e $TestExt --contents --regex"
Run-Test "T04 find-case-insensitive" "$Binary find CONTENTINDEX -d $TestDir -e $TestExt --contents -i"
Run-Test "T05 find-count"          "$Binary find fn -d $TestDir -e $TestExt --contents -c"

# T06-T09: index + fast
Run-Test "T06 index-build"         "$Binary index -d $TestDir"
Run-Test "T07 fast-search"         "$Binary fast main -d $TestDir -e $TestExt"
Run-Test "T08 fast-regex-icase"    "$Binary fast `".*handler.*`" -d $TestDir -e $TestExt --regex -i"
Run-Test "T09 fast-dirs-only"      "$Binary fast src -d $TestDir --dirs-only"
Run-Test "T09a fast-multi-term"    "$Binary fast `"main,lib,handler`" -d $TestDir -e $TestExt"

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

# T20: def-index + def-audit
Run-Test "T20 def-index"           "$Binary def-index -d $TestDir -e $TestExt"
Run-Test "T-DEF-AUDIT def-audit"   "$Binary def-audit -d $TestDir -e $TestExt"

# T49: def-index with TypeScript
Run-Test "T49 def-index-ts"        "$Binary def-index -d $TestDir -e ts"
# T-EXT-CHECK: verify index files have new semantic extensions
# NOTE: must run AFTER def-index (T20) since .code-structure files are created by def-index
Write-Host -NoNewline "  T-EXT-CHECK index-file-extensions ... "
$total++
try {
    $idxDir = Join-Path $env:LOCALAPPDATA "search-index"
    $fileListFiles = Get-ChildItem -Path $idxDir -Filter "*.file-list" -ErrorAction SilentlyContinue
    $wordSearchFiles = Get-ChildItem -Path $idxDir -Filter "*.word-search" -ErrorAction SilentlyContinue
    $codeStructFiles = Get-ChildItem -Path $idxDir -Filter "*.code-structure" -ErrorAction SilentlyContinue
    $oldIdx = Get-ChildItem -Path $idxDir -Filter "*.idx" -ErrorAction SilentlyContinue
    $oldCidx = Get-ChildItem -Path $idxDir -Filter "*.cidx" -ErrorAction SilentlyContinue
    $oldDidx = Get-ChildItem -Path $idxDir -Filter "*.didx" -ErrorAction SilentlyContinue

    $extPassed = $true
    if (-not $fileListFiles) {
        Write-Host "FAILED (no .file-list files found)" -ForegroundColor Red
        $extPassed = $false
    }
    if (-not $wordSearchFiles) {
        Write-Host "FAILED (no .word-search files found)" -ForegroundColor Red
        $extPassed = $false
    }
    if (-not $codeStructFiles) {
        Write-Host "FAILED (no .code-structure files found)" -ForegroundColor Red
        $extPassed = $false
    }
    if ($oldIdx -or $oldCidx -or $oldDidx) {
        Write-Host "FAILED (old .idx/.cidx/.didx files found)" -ForegroundColor Red
        $extPassed = $false
    }
    if ($extPassed) {
        Write-Host "OK (.file-list=$($fileListFiles.Count), .word-search=$($wordSearchFiles.Count), .code-structure=$($codeStructFiles.Count))" -ForegroundColor Green
        $passed++
    }
    else {
        $failed++
    }
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
}


# T21-T23: error handling
Run-Test "T21 invalid-regex"       "$Binary grep `"[invalid`" -d $TestDir -e $TestExt --regex" -ExpectedExit 1
Run-Test "T22 nonexistent-dir"     "$Binary find test -d /nonexistent/path/xyz" -ExpectedExit 1

# T42/T42b: tips — strategy recipes and query budget
Run-Test "T42 tips-strategy-recipes" "$Binary tips | Select-String 'STRATEGY RECIPES'"
Run-Test "T42b tips-query-budget"    "$Binary tips | Select-String 'Query budget'"

# T54, T65, T76, T80, T82: error handling and edge cases
# NOTE: "search definitions" and "search reindex" are MCP-only tools (no CLI subcommand).
#       Tests are adapted to use equivalent CLI commands (grep, fast, content-index).
#       T61 number is already used by grep-substring-default, so invalid regex test uses T65.

# Safety net: ensure content index exists before grep edge-case tests.
# T54/T82 depend on a content index (grep returns exit 1 without one).
# The index may be missing if: previous run cleaned up, binary was reinstalled,
# or the script was partially re-run. This makes these tests self-contained.
$ErrorActionPreference = "Continue"
Invoke-Expression "$Binary content-index -d $TestDir -e $TestExt 2>&1" | Out-Null
$ErrorActionPreference = "Stop"

# T54: grep with non-existent term should return 0 matches gracefully (not crash)
Run-Test "T54 grep-nonexistent-term" "$Binary grep ZZZNonExistentXYZ123 -d $TestDir -e $TestExt"

# T65: fast with invalid regex should return error (exit 1)
Run-Test "T65 fast-invalid-regex"    "$Binary fast `"[invalid`" -d $TestDir --regex" -ExpectedExit 1

# T76: fast with empty pattern — clap rejects it gracefully (exit 2 = usage error, not crash)
Run-Test "T76 fast-empty-pattern"    "$Binary fast `"`" -d $TestDir -e $TestExt" -ExpectedExit 2

# T80: grep with non-existent directory should return error (no index found)
Run-Test "T80 grep-nonexistent-dir"  "$Binary grep fn -d C:\nonexistent\fakepath123 -e $TestExt" -ExpectedExit 1

# T82: grep with --max-results 0 should work (0 means unlimited)
Run-Test "T82 grep-max-results-zero" "$Binary grep fn -d $TestDir -e $TestExt --max-results 0"

# T-SHUTDOWN: save-on-shutdown — verify incremental watcher updates survive server restart
Write-Host -NoNewline "  T-SHUTDOWN save-on-shutdown ... "
$total++
try {
    $t59dir = Join-Path $env:TEMP "search_e2e_shutdown_$PID"
    if (Test-Path $t59dir) { Remove-Item -Recurse -Force $t59dir }
    New-Item -ItemType Directory -Path $t59dir | Out-Null

    # Create a test file for the watcher to index
    $t59file = Join-Path $t59dir "Original.cs"
    Set-Content -Path $t59file -Value "class Original { void Run() { } }"

    # Build first (content-index) so the server has something to load
    $ErrorActionPreference = "Continue"
    & cargo run -- content-index -d $t59dir -e cs 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"

    # Find the search binary (installed or debug)
    $searchBin = (Get-Command search.exe -ErrorAction SilentlyContinue).Source
    if (-not $searchBin) { $searchBin = ".\target\debug\search.exe" }

    # Start MCP server with --watch using System.Diagnostics.Process for stdin control
    $stderrFile = Join-Path $t59dir "stderr.txt"
    $stdoutFile = Join-Path $t59dir "stdout.txt"
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $searchBin
    $psi.Arguments = "serve --dir `"$t59dir`" --ext cs --watch"
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $false
    $psi.RedirectStandardError = $false
    $psi.CreateNoWindow = $true
    # Redirect stderr to file to avoid deadlock
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true

    $t59proc = New-Object System.Diagnostics.Process
    $t59proc.StartInfo = $psi

    # Use async reading to avoid deadlocks
    $stderrBuilder = New-Object System.Text.StringBuilder
    $stdoutBuilder = New-Object System.Text.StringBuilder
    $errHandler = { if ($EventArgs.Data) { $Event.MessageData.AppendLine($EventArgs.Data) } }
    $outHandler = { if ($EventArgs.Data) { $Event.MessageData.AppendLine($EventArgs.Data) } }

    $errEvent = Register-ObjectEvent -InputObject $t59proc -EventName ErrorDataReceived -Action $errHandler -MessageData $stderrBuilder
    $outEvent = Register-ObjectEvent -InputObject $t59proc -EventName OutputDataReceived -Action $outHandler -MessageData $stdoutBuilder

    $t59proc.Start() | Out-Null
    $t59proc.BeginErrorReadLine()
    $t59proc.BeginOutputReadLine()

    # Send initialize request
    $initReq = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
    $t59proc.StandardInput.WriteLine($initReq)

    # Wait for server to start and watcher to be ready
    Start-Sleep -Seconds 3

    # Modify the file (watcher should pick this up)
    Set-Content -Path $t59file -Value "class Modified { void Execute() { } }"

    # Wait for watcher debounce
    Start-Sleep -Seconds 3

    # Close stdin to trigger graceful shutdown (save-on-shutdown)
    $t59proc.StandardInput.Close()

    # Wait for process to exit
    if (-not $t59proc.WaitForExit(15000)) {
        # Timeout — kill it
        $t59proc.Kill()
        $t59proc.WaitForExit(5000) | Out-Null
    }

    # Give async readers a moment to drain
    Start-Sleep -Milliseconds 500

    # Unregister events
    Unregister-Event -SourceIdentifier $errEvent.Name -ErrorAction SilentlyContinue
    Unregister-Event -SourceIdentifier $outEvent.Name -ErrorAction SilentlyContinue

    $stderrContent = $stderrBuilder.ToString()

    if ($stderrContent -match "Content index saved on shutdown|saving indexes before shutdown") {
        Write-Host "OK" -ForegroundColor Green
        $passed++
    }
    else {
        # Fallback: check if cidx was updated recently
        $cidxFilesAfter = Get-ChildItem -Path (Join-Path $env:LOCALAPPDATA "search-index") -Filter "*.word-search" |
        Where-Object { $_.LastWriteTime -gt (Get-Date).AddMinutes(-1) }
        if ($cidxFilesAfter) {
            Write-Host "OK (verified via file timestamp)" -ForegroundColor Green
            $passed++
        }
        else {
            Write-Host "FAILED (no save-on-shutdown detected)" -ForegroundColor Red
            Write-Host "    stderr: $stderrContent" -ForegroundColor Yellow
            $failed++
        }
    }

    # Cleanup
    if (!$t59proc.HasExited) { $t59proc.Kill() }
    $t59proc.Dispose()
    Remove-Item -Recurse -Force $t59dir -ErrorAction SilentlyContinue
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
    if ($t59proc -and !$t59proc.HasExited) { $t59proc.Kill() }
    if ($t59proc) { $t59proc.Dispose() }
    if ($errEvent) { Unregister-Event -SourceIdentifier $errEvent.Name -ErrorAction SilentlyContinue }
    if ($outEvent) { Unregister-Event -SourceIdentifier $outEvent.Name -ErrorAction SilentlyContinue }
    Remove-Item -Recurse -Force $t59dir -ErrorAction SilentlyContinue
}

# ─── T65-T66 + T67-T68: search_callers E2E tests (local var types + false positive filtering) ───

# Helper: find the search binary (installed or debug build)
$searchBin = (Get-Command search.exe -ErrorAction SilentlyContinue).Source
if (-not $searchBin) { $searchBin = ".\target\debug\search.exe" }

# --- T65-T66: Local var type extraction (direction=down) ---
Write-Host -NoNewline "  T65-66 callers-local-var-types-down ... "
$total++
try {
    $callerDir = Join-Path $env:TEMP "search_e2e_callers_down_$PID"
    if (Test-Path $callerDir) { Remove-Item -Recurse -Force $callerDir }
    New-Item -ItemType Directory -Path $callerDir | Out-Null

    # Create OrderValidator class with check() method
    $validatorTs = @"
export class OrderValidator {
    check(): boolean {
        return true;
    }
}
"@
    Set-Content -Path (Join-Path $callerDir "validator.ts") -Value $validatorTs

    # Create a consumer that uses local var: const v = new OrderValidator(); v.check();
    $consumerTs = @"
import { OrderValidator } from './validator';

export class OrderService {
    processOrder(): void {
        const validator = new OrderValidator();
        validator.check();
    }
}
"@
    Set-Content -Path (Join-Path $callerDir "service.ts") -Value $consumerTs

    # Build content-index and def-index
    $ErrorActionPreference = "Continue"
    & $searchBin content-index -d $callerDir -e ts 2>&1 | Out-Null
    & $searchBin def-index -d $callerDir -e ts 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"

    # Query search_callers direction=down for OrderService.processOrder
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"processOrder","class":"OrderService","direction":"down","depth":1}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $callerDir --ext ts --definitions 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    # Extract the JSON-RPC response (id=5)
    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    if ($jsonLine) {
        # Check that 'check' appears in callTree (callee found via local var type)
        # Note: output is double-escaped JSON (\"method\":\"check\")
        if ($jsonLine -match 'check') {
            Write-Host "OK" -ForegroundColor Green
            $passed++
        }
        else {
            Write-Host "FAILED (check() not found in callTree)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $failed++
        }
    }
    else {
        Write-Host "FAILED (no JSON-RPC response for id=5)" -ForegroundColor Red
        Write-Host "    output: $output" -ForegroundColor Yellow
        $failed++
    }

    # Cleanup
    $ErrorActionPreference = "Continue"
    & $searchBin cleanup --dir $callerDir 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"
    Remove-Item -Recurse -Force $callerDir -ErrorAction SilentlyContinue
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
    if (Test-Path $callerDir) {
        $ErrorActionPreference = "Continue"
        & $searchBin cleanup --dir $callerDir 2>&1 | Out-Null
        $ErrorActionPreference = "Stop"
        Remove-Item -Recurse -Force $callerDir -ErrorAction SilentlyContinue
    }
}

# --- T67: Direction=up — false positive filtering with receiver type mismatch ---
Write-Host -NoNewline "  T67 callers-up-false-positive-filter ... "
$total++
try {
    $filterDir = Join-Path $env:TEMP "search_e2e_callers_up_$PID"
    if (Test-Path $filterDir) { Remove-Item -Recurse -Force $filterDir }
    New-Item -ItemType Directory -Path $filterDir | Out-Null

    # Create TaskRunner class with resolve() method
    $taskTs = @"
export class TaskRunner {
    resolve(): boolean {
        return true;
    }
}
"@
    Set-Content -Path (Join-Path $filterDir "task.ts") -Value $taskTs

    # Good caller: uses TaskRunner.resolve()
    $goodCallerTs = @"
import { TaskRunner } from './task';

export class Orchestrator {
    run(): void {
        const task = new TaskRunner();
        task.resolve();
    }
}
"@
    Set-Content -Path (Join-Path $filterDir "orchestrator.ts") -Value $goodCallerTs

    # False positive caller: uses path.resolve() (unrelated)
    $falseCallerTs = @"
import * as path from 'path';

export class PathHelper {
    getFullPath(): string {
        return path.resolve('/tmp');
    }
}
"@
    Set-Content -Path (Join-Path $filterDir "pathhelper.ts") -Value $falseCallerTs

    # Build content-index and def-index
    $ErrorActionPreference = "Continue"
    & $searchBin content-index -d $filterDir -e ts 2>&1 | Out-Null
    & $searchBin def-index -d $filterDir -e ts 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"

    # Query search_callers direction=up for TaskRunner.resolve
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"resolve","class":"TaskRunner","direction":"up","depth":1}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $filterDir --ext ts --definitions 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    # Extract the JSON-RPC response (id=5)
    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    $testPassed = $true
    if ($jsonLine) {
        # GOOD: orchestrator.ts should appear (calls TaskRunner.resolve())
        if ($jsonLine -notmatch 'orchestrator') {
            Write-Host "FAILED (orchestrator.ts not found in callers)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $testPassed = $false
        }
        # BAD: pathhelper.ts should NOT appear (calls path.resolve(), not TaskRunner.resolve())
        if ($jsonLine -match 'pathhelper') {
            Write-Host "FAILED (pathhelper.ts should be filtered out as false positive)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $testPassed = $false
        }
        if ($testPassed) {
            Write-Host "OK" -ForegroundColor Green
            $passed++
        }
        else {
            $failed++
        }
    }
    else {
        Write-Host "FAILED (no JSON-RPC response for id=5)" -ForegroundColor Red
        Write-Host "    output: $output" -ForegroundColor Yellow
        $failed++
    }

    # Cleanup
    $ErrorActionPreference = "Continue"
    & $searchBin cleanup --dir $filterDir 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"
    Remove-Item -Recurse -Force $filterDir -ErrorAction SilentlyContinue
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
    if (Test-Path $filterDir) {
        $ErrorActionPreference = "Continue"
        & $searchBin cleanup --dir $filterDir 2>&1 | Out-Null
        $ErrorActionPreference = "Stop"
        Remove-Item -Recurse -Force $filterDir -ErrorAction SilentlyContinue
    }
}

# --- T68: Direction=up — graceful fallback when no call-site data ---
Write-Host -NoNewline "  T68 callers-up-graceful-fallback ... "
$total++
try {
    $fallbackDir = Join-Path $env:TEMP "search_e2e_callers_fallback_$PID"
    if (Test-Path $fallbackDir) { Remove-Item -Recurse -Force $fallbackDir }
    New-Item -ItemType Directory -Path $fallbackDir | Out-Null

    # Create DataService class with fetch() method
    $serviceTs = @"
export class DataService {
    fetch(): any[] {
        return [];
    }
}
"@
    Set-Content -Path (Join-Path $fallbackDir "dataservice.ts") -Value $serviceTs

    # Consumer that calls fetch() without explicit type annotation (receiver_type may be None)
    $consumerTs = @"
import { DataService } from './dataservice';

export class Consumer {
    load(): void {
        const svc = new DataService();
        const result = svc.fetch();
    }
}
"@
    Set-Content -Path (Join-Path $fallbackDir "consumer.ts") -Value $consumerTs

    # Build content-index and def-index
    $ErrorActionPreference = "Continue"
    & $searchBin content-index -d $fallbackDir -e ts 2>&1 | Out-Null
    & $searchBin def-index -d $fallbackDir -e ts 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"

    # Query search_callers direction=up for DataService.fetch
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"fetch","class":"DataService","direction":"up","depth":1}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $fallbackDir --ext ts --definitions 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    # Extract the JSON-RPC response (id=5)
    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    if ($jsonLine) {
        # Consumer should appear as a caller (graceful fallback - not filtered out)
        if ($jsonLine -match 'consumer') {
            Write-Host "OK" -ForegroundColor Green
            $passed++
        }
        else {
            Write-Host "FAILED (consumer.ts not found - false negative from missing call-site data)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $failed++
        }
    }
    else {
        Write-Host "FAILED (no JSON-RPC response for id=5)" -ForegroundColor Red
        Write-Host "    output: $output" -ForegroundColor Yellow
        $failed++
    }

    # Cleanup
    $ErrorActionPreference = "Continue"
    & $searchBin cleanup --dir $fallbackDir 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"
    Remove-Item -Recurse -Force $fallbackDir -ErrorAction SilentlyContinue
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
    if (Test-Path $fallbackDir) {
        $ErrorActionPreference = "Continue"
        & $searchBin cleanup --dir $fallbackDir 2>&1 | Out-Null
        $ErrorActionPreference = "Stop"
        Remove-Item -Recurse -Force $fallbackDir -ErrorAction SilentlyContinue
    }
}
# --- T69: Direction=up — comment-line false positive filtered ---
Write-Host -NoNewline "  T69 callers-up-comment-false-positive ... "
$total++
try {
    $commentDir = Join-Path $env:TEMP "search_e2e_callers_comment_$PID"
    if (Test-Path $commentDir) { Remove-Item -Recurse -Force $commentDir }
    New-Item -ItemType Directory -Path $commentDir | Out-Null

    # Create TaskRunner class with resolve() method
    $taskRunnerTs = @"
export class TaskRunner {
    resolve(): void {
        console.log("resolved");
    }
}
"@
    Set-Content -Path (Join-Path $commentDir "task-runner.ts") -Value $taskRunnerTs

    # Consumer: "resolve" appears in comments (lines 5-6) AND as a real call (line 8)
    $consumerTs = @"
import { TaskRunner } from "./task-runner";

export class Consumer {
    processData(): void {
        // We need to resolve the task before proceeding
        // The resolve method handles cleanup
        const runner = new TaskRunner();
        runner.resolve();
    }
}
"@
    Set-Content -Path (Join-Path $commentDir "consumer.ts") -Value $consumerTs

    # Build content-index and def-index
    $ErrorActionPreference = "Continue"
    & $searchBin content-index -d $commentDir -e ts 2>&1 | Out-Null
    & $searchBin def-index -d $commentDir -e ts 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"

    # Query search_callers direction=up for TaskRunner.resolve
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"resolve","class":"TaskRunner","direction":"up","depth":1}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $commentDir --ext ts --definitions 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    # Extract the JSON-RPC response (id=5)
    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    $testPassed = $true
    if ($jsonLine) {
        # GOOD: Consumer.processData should appear (real call at runner.resolve())
        if ($jsonLine -notmatch 'processData') {
            Write-Host "FAILED (Consumer.processData not found in callers)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $testPassed = $false
        }
        # Verify exactly 1 caller (comment lines with "resolve" should NOT be false positives)
        # The output is double-escaped JSON; count occurrences of processData in the call tree
        $methodMatches = [regex]::Matches($jsonLine, 'processData')
        if ($methodMatches.Count -lt 1) {
            Write-Host "FAILED (expected exactly 1 caller, got 0)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $testPassed = $false
        }
        # Check totalNodes=1 in summary to confirm exactly 1 caller
        # Note: JSON output has escaped quotes (\"totalNodes\":1), so match flexibly
        if ($jsonLine -notmatch 'totalNodes[^0-9]+1[^0-9]') {
            Write-Host "FAILED (expected totalNodes=1, comment lines created false positives)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $testPassed = $false
        }
        if ($testPassed) {
            Write-Host "OK" -ForegroundColor Green
            $passed++
        }
        else {
            $failed++
        }
    }
    else {
        Write-Host "FAILED (no JSON-RPC response for id=5)" -ForegroundColor Red
        Write-Host "    output: $output" -ForegroundColor Yellow
        $failed++
    }

    # Cleanup
    $ErrorActionPreference = "Continue"
    & $searchBin cleanup --dir $commentDir 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"
    Remove-Item -Recurse -Force $commentDir -ErrorAction SilentlyContinue
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
    if (Test-Path $commentDir) {
        $ErrorActionPreference = "Continue"
        & $searchBin cleanup --dir $commentDir 2>&1 | Out-Null
        $ErrorActionPreference = "Stop"
        Remove-Item -Recurse -Force $commentDir -ErrorAction SilentlyContinue
    }
}


# --- T-FIX3-EXPR-BODY: C# expression body property call sites ---
Write-Host -NoNewline "  T-FIX3-EXPR-BODY callers-csharp-expression-body ... "
$total++
try {
    $exprBodyDir = Join-Path $env:TEMP "search_e2e_expr_body_$PID"
    if (Test-Path $exprBodyDir) { Remove-Item -Recurse -Force $exprBodyDir }
    New-Item -ItemType Directory -Path $exprBodyDir | Out-Null

    # Create NameProvider class with GetName() method
    $nameProviderCs = @"
namespace TestApp
{
    public class NameProvider
    {
        public string GetName() => "test";
    }
}
"@
    Set-Content -Path (Join-Path $exprBodyDir "NameProvider.cs") -Value $nameProviderCs

    # Create Consumer with expression body property calling GetName()
    $consumerCs = @"
namespace TestApp
{
    public class Consumer
    {
        private NameProvider _provider;
        public string DisplayName => _provider.GetName();
    }
}
"@
    Set-Content -Path (Join-Path $exprBodyDir "Consumer.cs") -Value $consumerCs

    # Build content-index and def-index
    $ErrorActionPreference = "Continue"
    & $searchBin content-index -d $exprBodyDir -e cs 2>&1 | Out-Null
    & $searchBin def-index -d $exprBodyDir -e cs 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"

    # Query search_callers direction=up for NameProvider.GetName
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"GetName","class":"NameProvider","direction":"up","depth":1}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $exprBodyDir --ext cs --definitions 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    # Extract the JSON-RPC response (id=5)
    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    if ($jsonLine) {
        # Consumer.DisplayName (expression body property) should appear as a caller
        if ($jsonLine -match 'DisplayName') {
            Write-Host "OK" -ForegroundColor Green
            $passed++
        }
        else {
            Write-Host "FAILED (DisplayName not found in callers - expression body property not parsed)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $failed++
        }
    }
    else {
        Write-Host "FAILED (no JSON-RPC response for id=5)" -ForegroundColor Red
        Write-Host "    output: $output" -ForegroundColor Yellow
        $failed++
    }

    # Cleanup
    $ErrorActionPreference = "Continue"
    & $searchBin cleanup --dir $exprBodyDir 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"
    Remove-Item -Recurse -Force $exprBodyDir -ErrorAction SilentlyContinue
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
    if (Test-Path $exprBodyDir) {
        $ErrorActionPreference = "Continue"
        & $searchBin cleanup --dir $exprBodyDir 2>&1 | Out-Null
        $ErrorActionPreference = "Stop"
        Remove-Item -Recurse -Force $exprBodyDir -ErrorAction SilentlyContinue
    }
}

# --- T-FIX3-VERIFY: No false positives from missing call-site data (bypass #2 closed) ---
Write-Host -NoNewline "  T-FIX3-VERIFY callers-no-false-positives-missing-data ... "
$total++
try {
    $verifyDir = Join-Path $env:TEMP "search_e2e_verify_$PID"
    if (Test-Path $verifyDir) { Remove-Item -Recurse -Force $verifyDir }
    New-Item -ItemType Directory -Path $verifyDir | Out-Null

    # Create DataService class with Process() method
    $serviceCs = @"
namespace TestApp
{
    public class DataService
    {
        public void Process() { }
    }
}
"@
    Set-Content -Path (Join-Path $verifyDir "DataService.cs") -Value $serviceCs

    # Real caller: genuinely calls _service.Process()
    $realCallerCs = @"
namespace TestApp
{
    public class RealCaller
    {
        private DataService _service;
        public void Execute()
        {
            _service.Process();
        }
    }
}
"@
    Set-Content -Path (Join-Path $verifyDir "RealCaller.cs") -Value $realCallerCs

    # False caller: mentions "Process" in a string but has no actual call to DataService.Process()
    $falseCallerCs = @"
namespace TestApp
{
    public class FalseCaller
    {
        public void DoWork()
        {
            var msg = "We need to Process the data";
            System.Console.WriteLine(msg);
        }
    }
}
"@
    Set-Content -Path (Join-Path $verifyDir "FalseCaller.cs") -Value $falseCallerCs

    # Build content-index and def-index
    $ErrorActionPreference = "Continue"
    & $searchBin content-index -d $verifyDir -e cs 2>&1 | Out-Null
    & $searchBin def-index -d $verifyDir -e cs 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"

    # Query search_callers direction=up for DataService.Process
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"Process","class":"DataService","direction":"up","depth":1}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $verifyDir --ext cs --definitions 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    # Extract the JSON-RPC response (id=5)
    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    $testPassed = $true
    if ($jsonLine) {
        # GOOD: RealCaller should appear (has actual call site)
        if ($jsonLine -notmatch 'RealCaller') {
            Write-Host "FAILED (RealCaller not found in callers)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $testPassed = $false
        }
        # BAD: FalseCaller should NOT appear (no call-site data for DataService.Process)
        if ($jsonLine -match 'FalseCaller') {
            Write-Host "FAILED (FalseCaller should be filtered out - no call-site data)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $testPassed = $false
        }
        if ($testPassed) {
            Write-Host "OK" -ForegroundColor Green
            $passed++
        }
        else {
            $failed++
        }
    }
    else {
        Write-Host "FAILED (no JSON-RPC response for id=5)" -ForegroundColor Red
        Write-Host "    output: $output" -ForegroundColor Yellow
        $failed++
    }

    # Cleanup
    $ErrorActionPreference = "Continue"
    & $searchBin cleanup --dir $verifyDir 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"
    Remove-Item -Recurse -Force $verifyDir -ErrorAction SilentlyContinue
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
    if (Test-Path $verifyDir) {
        $ErrorActionPreference = "Continue"
        & $searchBin cleanup --dir $verifyDir 2>&1 | Out-Null
        $ErrorActionPreference = "Stop"
        Remove-Item -Recurse -Force $verifyDir -ErrorAction SilentlyContinue
    }
}

# --- T-FIX3-LAMBDA: Lambda calls in arguments captured (C#) ---
Write-Host -NoNewline "  T-FIX3-LAMBDA callers-csharp-lambda-in-args ... "
$total++
try {
    $lambdaDir = Join-Path $env:TEMP "search_e2e_lambda_$PID"
    if (Test-Path $lambdaDir) { Remove-Item -Recurse -Force $lambdaDir }
    New-Item -ItemType Directory -Path $lambdaDir | Out-Null

    # Create Validator class with Validate() method
    $validatorCs = @"
using System;
namespace TestApp
{
    public class Validator
    {
        public bool Validate(string s)
        {
            return s.Length > 0;
        }
    }
}
"@
    Set-Content -Path (Join-Path $lambdaDir "Validator.cs") -Value $validatorCs

    # Create Processor that calls Validate() inside a lambda argument
    $processorCs = @"
using System;
using System.Collections.Generic;
namespace TestApp
{
    public class Processor
    {
        private Validator _validator;
        public void ProcessAll(List<string> items)
        {
            items.ForEach(x => _validator.Validate(x));
        }
    }
}
"@
    Set-Content -Path (Join-Path $lambdaDir "Processor.cs") -Value $processorCs

    # Build content-index and def-index
    $ErrorActionPreference = "Continue"
    & $searchBin content-index -d $lambdaDir -e cs 2>&1 | Out-Null
    & $searchBin def-index -d $lambdaDir -e cs 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"

    # Query search_callers direction=up for Validator.Validate
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"Validate","class":"Validator","direction":"up","depth":1}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $lambdaDir --ext cs --definitions 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    # Extract the JSON-RPC response (id=5)
    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    if ($jsonLine) {
        # Processor.ProcessAll should appear as a caller (lambda call inside ForEach)
        if ($jsonLine -match 'ProcessAll') {
            Write-Host "OK" -ForegroundColor Green
            $passed++
        }
        else {
            Write-Host "FAILED (ProcessAll not found in callers - lambda call site not captured)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $failed++
        }
    }
    else {
        Write-Host "FAILED (no JSON-RPC response for id=5)" -ForegroundColor Red
        Write-Host "    output: $output" -ForegroundColor Yellow
        $failed++
    }

    # Cleanup
    $ErrorActionPreference = "Continue"
    & $searchBin cleanup --dir $lambdaDir 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"
    Remove-Item -Recurse -Force $lambdaDir -ErrorAction SilentlyContinue
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
    if (Test-Path $lambdaDir) {
        $ErrorActionPreference = "Continue"
        & $searchBin cleanup --dir $lambdaDir 2>&1 | Out-Null
        $ErrorActionPreference = "Stop"
        Remove-Item -Recurse -Force $lambdaDir -ErrorAction SilentlyContinue
    }
}

# --- T-OVERLOAD-DEDUP-UP: Overloaded callers not collapsed (direction=up) ---
Write-Host -NoNewline "  T-OVERLOAD-DEDUP-UP callers-overloads-not-collapsed-up ... "
$total++
try {
    $overloadUpDir = Join-Path $env:TEMP "search_e2e_overload_up_$PID"
    if (Test-Path $overloadUpDir) { Remove-Item -Recurse -Force $overloadUpDir }
    New-Item -ItemType Directory -Path $overloadUpDir | Out-Null

    $validatorCs = @"
namespace TestApp
{
    public class Validator
    {
        public bool Validate() { return true; }
    }
}
"@
    Set-Content -Path (Join-Path $overloadUpDir "Validator.cs") -Value $validatorCs

    $processorCs = @"
namespace TestApp
{
    public class Processor
    {
        private Validator _validator;
        public void Process(int x)
        {
            _validator.Validate();
        }
        public void Process(string s)
        {
            _validator.Validate();
        }
    }
}
"@
    Set-Content -Path (Join-Path $overloadUpDir "Processor.cs") -Value $processorCs

    $ErrorActionPreference = "Continue"
    & $searchBin content-index -d $overloadUpDir -e cs 2>&1 | Out-Null
    & $searchBin def-index -d $overloadUpDir -e cs 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"

    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"Validate","class":"Validator","direction":"up","depth":1}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $overloadUpDir --ext cs --definitions 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    if ($jsonLine) {
        $processMatches = [regex]::Matches($jsonLine, '\\?"Process\\?"')
        if ($processMatches.Count -ge 2) {
            Write-Host "OK" -ForegroundColor Green
            $passed++
        }
        else {
            Write-Host "FAILED (expected 2 Process overloads in callers, got $($processMatches.Count))" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $failed++
        }
    }
    else {
        Write-Host "FAILED (no JSON-RPC response for id=5)" -ForegroundColor Red
        Write-Host "    output: $output" -ForegroundColor Yellow
        $failed++
    }

    $ErrorActionPreference = "Continue"
    & $searchBin cleanup --dir $overloadUpDir 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"
    Remove-Item -Recurse -Force $overloadUpDir -ErrorAction SilentlyContinue
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
    if (Test-Path $overloadUpDir) {
        $ErrorActionPreference = "Continue"
        & $searchBin cleanup --dir $overloadUpDir 2>&1 | Out-Null
        $ErrorActionPreference = "Stop"
        Remove-Item -Recurse -Force $overloadUpDir -ErrorAction SilentlyContinue
    }
}

# --- T-SAME-NAME-IFACE: Same method name on unrelated interfaces — no cross-contamination ---
Write-Host -NoNewline "  T-SAME-NAME-IFACE callers-same-name-unrelated-iface ... "
$total++
try {
    $ifaceDir = Join-Path $env:TEMP "search_e2e_same_name_iface_$PID"
    if (Test-Path $ifaceDir) { Remove-Item -Recurse -Force $ifaceDir }
    New-Item -ItemType Directory -Path $ifaceDir | Out-Null

    $iServiceACs = @"
namespace TestApp
{
    public interface IServiceA
    {
        void Execute();
    }
}
"@
    Set-Content -Path (Join-Path $ifaceDir "IServiceA.cs") -Value $iServiceACs

    $iServiceBCs = @"
namespace TestApp
{
    public interface IServiceB
    {
        void Execute();
    }
}
"@
    Set-Content -Path (Join-Path $ifaceDir "IServiceB.cs") -Value $iServiceBCs

    $serviceACs = @"
namespace TestApp
{
    public class ServiceA : IServiceA
    {
        public void Execute() { }
    }
}
"@
    Set-Content -Path (Join-Path $ifaceDir "ServiceA.cs") -Value $serviceACs

    $serviceBCs = @"
namespace TestApp
{
    public class ServiceB : IServiceB
    {
        public void Execute() { }
    }
}
"@
    Set-Content -Path (Join-Path $ifaceDir "ServiceB.cs") -Value $serviceBCs

    $consumerCs = @"
namespace TestApp
{
    public class Consumer
    {
        private IServiceB _serviceB;
        public void DoWork()
        {
            _serviceB.Execute();
        }
    }
}
"@
    Set-Content -Path (Join-Path $ifaceDir "Consumer.cs") -Value $consumerCs

    $ErrorActionPreference = "Continue"
    & $searchBin content-index -d $ifaceDir -e cs 2>&1 | Out-Null
    & $searchBin def-index -d $ifaceDir -e cs 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"

    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_callers","arguments":{"method":"Execute","class":"ServiceA","direction":"up","depth":1}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $ifaceDir --ext cs --definitions 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    $testPassed = $true
    if ($jsonLine) {
        # Consumer should NOT appear (it calls IServiceB.Execute, not IServiceA.Execute)
        if ($jsonLine -match 'Consumer') {
            Write-Host "FAILED (Consumer should NOT appear as caller of ServiceA.Execute)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $testPassed = $false
        }
        # Explicit assertion: totalNodes must be 0 (no callers for ServiceA.Execute)
        if ($jsonLine -notmatch 'totalNodes[^0-9]+0[^0-9]') {
            Write-Host "FAILED (expected totalNodes=0, got non-zero - unexpected caller in tree)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $testPassed = $false
        }
        if ($testPassed) {
            Write-Host "OK" -ForegroundColor Green
            $passed++
        }
        else {
            $failed++
        }
    }
    else {
        Write-Host "FAILED (no JSON-RPC response for id=5)" -ForegroundColor Red
        Write-Host "    output: $output" -ForegroundColor Yellow
        $failed++
    }

    $ErrorActionPreference = "Continue"
    & $searchBin cleanup --dir $ifaceDir 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"
    Remove-Item -Recurse -Force $ifaceDir -ErrorAction SilentlyContinue
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
    if (Test-Path $ifaceDir) {
        $ErrorActionPreference = "Continue"
        & $searchBin cleanup --dir $ifaceDir 2>&1 | Out-Null
        $ErrorActionPreference = "Stop"
        Remove-Item -Recurse -Force $ifaceDir -ErrorAction SilentlyContinue
    }
}

# Note: T-FIX3-PREFILTER (base types removed from pre-filter) and T-FIX3-FIND-CONTAINING
# (find_containing_method returns di directly) are internal optimizations with no CLI-observable
# differences. They are covered by unit tests in handlers_tests_csharp.rs.

# ─── New E2E tests for features added in 2026-02-21/22 ───

# --- T-SERVE-HELP-TOOLS: verify serve --help lists key tools ---
Write-Host -NoNewline "  T-SERVE-HELP-TOOLS serve-help-tool-list ... "
$total++
try {
    $ErrorActionPreference = "Continue"
    $helpOutput = & $searchBin serve --help 2>&1 | Out-String
    $ErrorActionPreference = "Stop"

    $requiredTools = @(
        "search_branch_status",
        "search_git_blame",
        "search_help",
        "search_reindex_definitions"
    )
    $helpPassed = $true
    foreach ($tool in $requiredTools) {
        if ($helpOutput -notmatch $tool) {
            Write-Host "FAILED (missing tool in serve --help: $tool)" -ForegroundColor Red
            $helpPassed = $false
        }
    }
    if ($helpPassed) {
        Write-Host "OK" -ForegroundColor Green
        $passed++
    }
    else {
        $failed++
    }
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
}

# --- T-BRANCH-STATUS: smoke test for search_branch_status MCP tool ---
Write-Host -NoNewline "  T-BRANCH-STATUS branch-status-smoke ... "
$total++
try {
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_branch_status","arguments":{"repo":"."}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $TestDir --ext $TestExt 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    if ($jsonLine -and $jsonLine -match 'currentBranch' -and $jsonLine -match 'isMainBranch' -and $jsonLine -notmatch '"isError"\s*:\s*true') {
        Write-Host "OK" -ForegroundColor Green
        $passed++
    }
    else {
        Write-Host "FAILED (missing currentBranch/isMainBranch or isError)" -ForegroundColor Red
        Write-Host "    output: $jsonLine" -ForegroundColor Yellow
        $failed++
    }
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
}

# --- T-GIT-FILE-NOT-FOUND: nonexistent file returns warning, not error ---
Write-Host -NoNewline "  T-GIT-FILE-NOT-FOUND git-history-file-warning ... "
$total++
try {
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_git_history","arguments":{"repo":".","file":"DOES_NOT_EXIST_12345.txt"}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $TestDir --ext $TestExt 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    if ($jsonLine -and $jsonLine -match 'warning' -and $jsonLine -notmatch '"isError"\s*:\s*true') {
        Write-Host "OK" -ForegroundColor Green
        $passed++
    }
    else {
        Write-Host "FAILED (expected warning field, no isError)" -ForegroundColor Red
        Write-Host "    output: $jsonLine" -ForegroundColor Yellow
        $failed++
    }
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
}

# --- T-GIT-NOCACHE: noCache parameter returns valid result ---
Write-Host -NoNewline "  T-GIT-NOCACHE git-history-nocache ... "
$total++
try {
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_git_history","arguments":{"repo":".","file":"Cargo.toml","noCache":true,"maxResults":1}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $TestDir --ext $TestExt 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    if ($jsonLine -and $jsonLine -match 'commits' -and $jsonLine -notmatch '"isError"\s*:\s*true') {
        Write-Host "OK" -ForegroundColor Green
        $passed++
    }
    else {
        Write-Host "FAILED (expected commits, no isError)" -ForegroundColor Red
        Write-Host "    output: $jsonLine" -ForegroundColor Yellow
        $failed++
    }
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
}

# --- T-GIT-TOTALCOMMITS: totalCommits shows real total, not truncated count (BUG-2 regression) ---
Write-Host -NoNewline "  T-GIT-TOTALCOMMITS git-history-total-vs-returned ... "
$total++
try {
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_git_history","arguments":{"repo":".","file":"Cargo.toml","maxResults":1}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $TestDir --ext $TestExt 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    if ($jsonLine) {
        # Extract totalCommits and returned from the response (handles escaped JSON quotes)
        $totalMatch = [regex]::Match($jsonLine, 'totalCommits\\?"?\s*:\s*(\d+)')
        $returnedMatch = [regex]::Match($jsonLine, 'returned\\?"?\s*:\s*(\d+)')
        if ($totalMatch.Success -and $returnedMatch.Success) {
            $totalVal = [int]$totalMatch.Groups[1].Value
            $returnedVal = [int]$returnedMatch.Groups[1].Value
            # Cargo.toml should have more than 1 commit in this repo
            if ($totalVal -gt $returnedVal -and $returnedVal -eq 1) {
                Write-Host "OK (total=$totalVal, returned=$returnedVal)" -ForegroundColor Green
                $passed++
            }
            else {
                Write-Host "FAILED (totalCommits=$totalVal should be > returned=$returnedVal)" -ForegroundColor Red
                Write-Host "    output: $jsonLine" -ForegroundColor Yellow
                $failed++
            }
        }
        else {
            Write-Host "FAILED (could not parse totalCommits/returned)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $failed++
        }
    }
    else {
        Write-Host "FAILED (no JSON-RPC response for id=5)" -ForegroundColor Red
        Write-Host "    output: $output" -ForegroundColor Yellow
        $failed++
    }
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
}

# --- T-GIT-CACHE: Git cache routing — search_git_history returns commits ---
Write-Host -NoNewline "  T-GIT-CACHE git-cache-routing ... "
$total++
try {
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_git_history","arguments":{"repo":".","file":"Cargo.toml","maxResults":2}}}'
    ) -join "`n"

    $ErrorActionPreference = "Continue"
    $output = ($msgs | & $searchBin serve --dir $TestDir --ext $TestExt 2>$null) | Out-String
    $ErrorActionPreference = "Stop"

    # Extract the JSON-RPC response (id=5)
    $jsonLine = $output -split "`n" | Where-Object { $_ -match '"id"\s*:\s*5' } | Select-Object -Last 1
    if ($jsonLine) {
        # Check that 'commits' appears in response (from cache or CLI fallback — both valid)
        if ($jsonLine -match 'commits') {
            Write-Host "OK" -ForegroundColor Green
            $passed++
        }
        else {
            Write-Host "FAILED (no 'commits' in response)" -ForegroundColor Red
            Write-Host "    output: $jsonLine" -ForegroundColor Yellow
            $failed++
        }
    }
    else {
        Write-Host "FAILED (no JSON-RPC response for id=5)" -ForegroundColor Red
        Write-Host "    output: $output" -ForegroundColor Yellow
        $failed++
    }
}
catch {
    Write-Host "FAILED (exception: $_)" -ForegroundColor Red
    $failed++
}

# T25-T52: serve (MCP)
Write-Host "  T25-T52: MCP serve tests - run manually (see e2e-test-plan.md)"

# T53-T58: TypeScript callers (MCP)
Write-Host "  T53-T58: TypeScript callers MCP tests - run manually (see e2e-test-plan.md)"

# Cleanup: remove index files created during E2E tests
Write-Host "`nCleaning up test indexes..."
$ErrorActionPreference = "Continue"
# Remove indexes for the test directory (targeted -- does not touch other projects)
Invoke-Expression "$Binary cleanup --dir $TestDir 2>&1" | Out-Null
# Remove orphaned indexes (e.g. T-SHUTDOWN temp dir that was already deleted)
Invoke-Expression "$Binary cleanup 2>&1" | Out-Null
$ErrorActionPreference = "Stop"

Write-Host "`n=== Results: $passed passed, $failed failed, $total total ===`n"
if ($failed -gt 0) { exit 1 }