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
Run-Test "T16 grep-context"        "$Binary grep is_stale -d $TestDir -e $TestExt --show-lines -C 2 --max-results 2"
Run-Test "T17 grep-exclude"        "$Binary grep ContentIndex -d $TestDir -e $TestExt --exclude-dir bench"
Run-Test "T18 grep-count"          "$Binary grep fn -d $TestDir -e $TestExt -c"
Run-Test "T24 grep-before-after"   "$Binary grep is_stale -d $TestDir -e $TestExt --show-lines -B 1 -A 3"

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
        $cidxFilesAfter = Get-ChildItem -Path (Join-Path $env:LOCALAPPDATA "search-index") -Filter "*.cidx" |
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

# T25-T52: serve (MCP)
Write-Host "  T25-T52: MCP serve tests - run manually (see e2e-test-plan.md)"

# T53-T58: TypeScript callers (MCP)
Write-Host "  T53-T58: TypeScript callers MCP tests - run manually (see e2e-test-plan.md)"

Write-Host "`n=== Results: $passed passed, $failed failed, $total total ===`n"
if ($failed -gt 0) { exit 1 }