#!/usr/bin/env pwsh
param(
    [string]$TestDir = ".",
    [string]$TestExt = "rs",
    [string]$Binary = "search"
)

$ErrorActionPreference = "Continue"
$passed = 0
$failed = 0
$total = 0

function Run-Test {
    param([string]$Name, [string]$Command, [int]$ExpectedExit = 0, [string]$StdoutContains = "")

    $script:total++
    Write-Host -NoNewline "  $Name ... "

    try {
        $output = cmd /c "$Command 2>&1"
        $exitCode = $LASTEXITCODE
    }
    catch {
        $output = $_.Exception.Message
        $exitCode = 1
    }

    $outputStr = $output -join "`n"

    if ($exitCode -ne $ExpectedExit) {
        Write-Host "FAILED (exit=$exitCode, expected=$ExpectedExit)" -ForegroundColor Red
        $preview = $outputStr.Substring(0, [Math]::Min(500, $outputStr.Length))
        Write-Host "    Output: $preview" -ForegroundColor DarkGray
        $script:failed++
        return
    }

    if ($StdoutContains -and -not ($outputStr -match [regex]::Escape($StdoutContains))) {
        Write-Host "FAILED (output missing: $StdoutContains)" -ForegroundColor Red
        $preview = $outputStr.Substring(0, [Math]::Min(500, $outputStr.Length))
        Write-Host "    Output: $preview" -ForegroundColor DarkGray
        $script:failed++
        return
    }

    Write-Host "OK" -ForegroundColor Green
    $script:passed++
}

Write-Host "`n=== E2E Tests (dir=$TestDir, ext=$TestExt, binary=$Binary) ===`n"

# T01-T05: find
Run-Test "T01 find-filename"         "$Binary find main -d $TestDir -e $TestExt"
Run-Test "T02 find-contents"         "$Binary find `"fn main`" -d $TestDir -e $TestExt --contents"
Run-Test "T03 find-regex"            "$Binary find `"fn\s+\w+`" -d $TestDir -e $TestExt --contents --regex"
Run-Test "T04 find-case-insensitive" "$Binary find CONTENTINDEX -d $TestDir -e $TestExt --contents -i"
Run-Test "T05 find-count"            "$Binary find fn -d $TestDir -e $TestExt --contents -c"

# T06-T09: index + fast
Run-Test "T06 index-build"           "$Binary index -d $TestDir"
Run-Test "T07 fast-search"           "$Binary fast main -d $TestDir -e $TestExt"
Run-Test "T08 fast-regex-icase"      "$Binary fast `".*handler.*`" -d $TestDir -e $TestExt --regex -i"
Run-Test "T09 fast-dirs-only"        "$Binary fast `"`" -d $TestDir --dirs-only"
Run-Test "T09a fast-multi-term"      "$Binary fast `"main,lib,index`" -d $TestDir -e $TestExt"

# T10: content-index
Run-Test "T10 content-index"         "$Binary content-index -d $TestDir -e $TestExt"

# T11-T18: grep
Run-Test "T11 grep-single"           "$Binary grep tokenize -d $TestDir -e $TestExt"
Run-Test "T12 grep-multi-or"         "$Binary grep `"tokenize,posting`" -d $TestDir -e $TestExt"
Run-Test "T13 grep-multi-and"        "$Binary grep `"tokenize,posting`" -d $TestDir -e $TestExt --all"
Run-Test "T14 grep-regex"            "$Binary grep `".*stale.*`" -d $TestDir -e $TestExt --regex"
Run-Test "T15 grep-phrase"           "$Binary grep `"pub fn`" -d $TestDir -e $TestExt --phrase"
Run-Test "T16 grep-context"          "$Binary grep is_stale -d $TestDir -e $TestExt --show-lines -C 2 --max-results 2"
Run-Test "T17 grep-exclude"          "$Binary grep ContentIndex -d $TestDir -e $TestExt --exclude-dir bench"
Run-Test "T18 grep-count"            "$Binary grep fn -d $TestDir -e $TestExt -c"
Run-Test "T24 grep-before-after"     "$Binary grep is_stale -d $TestDir -e $TestExt --show-lines -B 1 -A 3"

# T19: info
Run-Test "T19 info"                  "$Binary info"

# T20: def-index
Run-Test "T20 def-index"             "$Binary def-index -d $TestDir -e $TestExt"

# T21-T23: error handling
Run-Test "T21 invalid-regex"         "$Binary grep `"[invalid`" -d $TestDir -e $TestExt --regex" -ExpectedExit 1
Run-Test "T22 nonexistent-dir"       "$Binary find test -d /nonexistent/path/xyz" -ExpectedExit 1
Run-Test "T23 grep-no-index"         "$Binary grep test -d /tmp/empty_dir_no_index_xyz -e xyz" -ExpectedExit 1

# T25: serve — MCP initialize
$script:total++
Write-Host -NoNewline "  T25 serve-initialize ... "
try {
    $initMsg = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}'
    $output = echo $initMsg | cmd /c "$Binary serve --dir $TestDir --ext $TestExt 2>NUL"
    $outputStr = $output -join "`n"
    if ($outputStr -match '"serverInfo"') {
        Write-Host "OK" -ForegroundColor Green
        $script:passed++
    }
    else {
        Write-Host "FAILED (no serverInfo in response)" -ForegroundColor Red
        $script:failed++
    }
}
catch {
    Write-Host "FAILED ($($_.Exception.Message))" -ForegroundColor Red
    $script:failed++
}

# T26: serve — tools/list
$script:total++
Write-Host -NoNewline "  T26 serve-tools-list ... "
try {
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
    ) -join "`n"
    $output = echo $msgs | cmd /c "$Binary serve --dir $TestDir --ext $TestExt 2>NUL"
    $outputStr = $output -join "`n"
    if ($outputStr -match 'search_grep' -and $outputStr -match 'search_callers' -and $outputStr -match 'search_help') {
        Write-Host "OK" -ForegroundColor Green
        $script:passed++
    }
    else {
        Write-Host "FAILED (missing tools in response)" -ForegroundColor Red
        $script:failed++
    }
}
catch {
    Write-Host "FAILED ($($_.Exception.Message))" -ForegroundColor Red
    $script:failed++
}

# T27: serve — search_grep via tools/call
$script:total++
Write-Host -NoNewline "  T27 serve-grep ... "
try {
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"tokenize"}}}'
    ) -join "`n"
    $output = echo $msgs | cmd /c "$Binary serve --dir $TestDir --ext $TestExt 2>NUL"
    $outputStr = $output -join "`n"
    if ($outputStr -match 'totalFiles') {
        Write-Host "OK" -ForegroundColor Green
        $script:passed++
    }
    else {
        Write-Host "FAILED (no totalFiles in response)" -ForegroundColor Red
        $script:failed++
    }
}
catch {
    Write-Host "FAILED ($($_.Exception.Message))" -ForegroundColor Red
    $script:failed++
}

# T30: serve — search_help
$script:total++
Write-Host -NoNewline "  T30 serve-help ... "
try {
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_help","arguments":{}}}'
    ) -join "`n"
    $output = echo $msgs | cmd /c "$Binary serve --dir $TestDir --ext $TestExt 2>NUL"
    $outputStr = $output -join "`n"
    if ($outputStr -match 'bestPractices') {
        Write-Host "OK" -ForegroundColor Green
        $script:passed++
    }
    else {
        Write-Host "FAILED (no best_practices in response)" -ForegroundColor Red
        $script:failed++
    }
}
catch {
    Write-Host "FAILED ($($_.Exception.Message))" -ForegroundColor Red
    $script:failed++
}

# T42: serve — response size truncation for broad queries
# Uses maxResults:0 + showLines + contextLines:5 to guarantee response > 32KB before truncation
$script:total++
Write-Host -NoNewline "  T42 serve-response-truncation ... "
try {
    $msgs = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_grep","arguments":{"terms":"fn","substring":true,"maxResults":0,"showLines":true,"contextLines":5}}}'
    ) -join "`n"
    $output = echo $msgs | cmd /c "$Binary serve --dir $TestDir --ext $TestExt --metrics 2>NUL"
    $outputStr = $output -join "`n"

    # Parse the tools/call response (skip initialize response)
    $responses = $outputStr -split "`n" | Where-Object { $_ -match '"id"' }
    $grepResponse = $responses | Where-Object { $_ -match '"id"\s*:\s*2' } | Select-Object -First 1

    if ($grepResponse -and $grepResponse -match 'responseTruncated') {
        Write-Host "OK (truncation active, size=$($grepResponse.Length) bytes)" -ForegroundColor Green
        $script:passed++
    }
    else {
        Write-Host "FAILED (responseTruncated not found in response)" -ForegroundColor Red
        if ($grepResponse) {
            $preview = $grepResponse.Substring(0, [Math]::Min(300, $grepResponse.Length))
            Write-Host "    Response: $preview" -ForegroundColor DarkGray
        }
        $script:failed++
    }
}
catch {
    Write-Host "FAILED ($($_.Exception.Message))" -ForegroundColor Red
    $script:failed++
}

# ASCII-safety test: verify all CLI output is pure ASCII (no Unicode box-drawing, emoji, etc.)
# Windows cmd.exe cannot display non-ASCII characters correctly.
function Test-AsciiSafe {
    param([string]$Name, [string]$Command)

    $script:total++
    Write-Host -NoNewline "  $Name ... "

    try {
        $output = cmd /c "$Command 2>&1"
        $outputStr = $output -join "`n"

        $nonAscii = [regex]::Matches($outputStr, '[^\x00-\x7F]')
        if ($nonAscii.Count -gt 0) {
            $chars = ($nonAscii | Select-Object -First 5 | ForEach-Object {
                    "U+$([string]::Format('{0:X4}', [int][char]$_.Value)) '$($_.Value)'"
                }) -join ", "
            Write-Host "FAILED ($($nonAscii.Count) non-ASCII chars: $chars)" -ForegroundColor Red
            $script:failed++
        }
        else {
            Write-Host "OK" -ForegroundColor Green
            $script:passed++
        }
    }
    catch {
        Write-Host "FAILED ($($_.Exception.Message))" -ForegroundColor Red
        $script:failed++
    }
}

Test-AsciiSafe "ASCII-safe: tips"      "$Binary tips"
Test-AsciiSafe "ASCII-safe: info"      "$Binary info"
Test-AsciiSafe "ASCII-safe: grep"      "$Binary grep tokenize -d $TestDir -e $TestExt"
Test-AsciiSafe "ASCII-safe: find"      "$Binary find main -d $TestDir -e $TestExt"
Test-AsciiSafe "ASCII-safe: fast"      "$Binary fast main -d $TestDir -e $TestExt"
Test-AsciiSafe "ASCII-safe: help"      "$Binary --help"
Test-AsciiSafe "ASCII-safe: grep-help" "$Binary grep --help"
Test-AsciiSafe "ASCII-safe: serve-help" "$Binary serve --help"
Test-AsciiSafe "ASCII-safe: def-index-help" "$Binary def-index --help"

Write-Host "`n=== Results: $passed passed, $failed failed, $total total ===`n"
if ($failed -gt 0) { exit 1 }