#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Regression tests for setup-xray.ps1 runtime dependency handling.

.DESCRIPTION
    Verifies that public downloads do not require gh, Git-managed repositories
    fail before mutation when git is unusable, and tracked Hidden installs
    validate their real clean/smudge runtime before changing MCP config files.
#>

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$Script:Passed = 0
$Script:Failed = 0

function Assert-True {
    param(
        [Parameter(Mandatory)] [bool]$Condition,
        [Parameter(Mandatory)] [string]$Label
    )

    if ($Condition) {
        $Script:Passed++
        Write-Host "PASS  $Label" -ForegroundColor Green
    }
    else {
        $Script:Failed++
        Write-Host "FAIL  $Label" -ForegroundColor Red
    }
}

function Assert-Equal {
    param(
        $Actual,
        $Expected,
        [Parameter(Mandatory)] [string]$Label
    )

    Assert-True -Condition ($Actual -eq $Expected) -Label $Label
    if ($Actual -ne $Expected) {
        Write-Host "      expected: $Expected" -ForegroundColor DarkGray
        Write-Host "      actual:   $Actual" -ForegroundColor DarkGray
    }
}

function Assert-BytesEqual {
    param(
        [Parameter(Mandatory)] [byte[]]$Actual,
        [Parameter(Mandatory)] [byte[]]$Expected,
        [Parameter(Mandatory)] [string]$Label
    )

    $actualBase64 = [Convert]::ToBase64String($Actual)
    $expectedBase64 = [Convert]::ToBase64String($Expected)
    Assert-Equal -Actual $actualBase64 -Expected $expectedBase64 -Label $Label
}

function Write-FakeWindowsExecutable {
    param([Parameter(Mandatory)] [string]$Path)

    $bytes = New-Object byte[] 68
    $bytes[0] = 0x4D
    $bytes[1] = 0x5A
    [BitConverter]::GetBytes(64).CopyTo($bytes, 0x3C)
    $bytes[64] = 0x50
    $bytes[65] = 0x45
    [IO.File]::WriteAllBytes($Path, $bytes)
}

function New-TempDirectory {
    $path = Join-Path ([IO.Path]::GetTempPath()) ("xray-dependency-test-{0}" -f [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $path -Force | Out-Null
    return $path
}

function New-InstallDirectory {
    param([Parameter(Mandatory)] [string]$Root)

    $path = Join-Path $Root ("install-{0}" -f [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $path -Force | Out-Null
    New-Item -ItemType File -Path (Join-Path $path 'xray.exe') -Force | Out-Null
    return $path
}

function New-GitFixture {
    param(
        [Parameter(Mandatory)] [string]$Root,
        [Parameter(Mandatory)] [string]$Name,
        [ValidateSet('None', 'VsCode', 'CopilotCli')] [string]$Config = 'None'
    )

    $repo = Join-Path $Root $Name
    New-Item -ItemType Directory -Path $repo -Force | Out-Null
    & $Script:GitExe -C $repo init --quiet
    & $Script:GitExe -C $repo config user.name 'Dependency Test'
    & $Script:GitExe -C $repo config user.email 'dependency-test@example.invalid'
    & $Script:GitExe -C $repo config core.autocrlf true
    & $Script:GitExe -C $repo config core.safecrlf true

    if ($Config -eq 'VsCode') {
        $configPath = Join-Path $repo '.vscode/mcp.json'
        New-Item -ItemType Directory -Path (Split-Path -Parent $configPath) -Force | Out-Null
        [IO.File]::WriteAllText($configPath, "{`r`n  `"servers`": {`r`n  }`r`n}`r`n", [Text.UTF8Encoding]::new($false))
        & $Script:GitExe -C $repo -c core.safecrlf=false add '.vscode/mcp.json'
    }
    elseif ($Config -eq 'CopilotCli') {
        $configPath = Join-Path $repo '.mcp.json'
        [IO.File]::WriteAllText($configPath, "{`r`n  `"mcpServers`": {`r`n  }`r`n}`r`n", [Text.UTF8Encoding]::new($false))
        & $Script:GitExe -C $repo -c core.safecrlf=false add '.mcp.json'
    }
    else {
        Set-Content -Path (Join-Path $repo 'source.ts') -Value 'export const value = 1;' -Encoding UTF8
        & $Script:GitExe -C $repo -c core.safecrlf=false add 'source.ts'
    }

    & $Script:GitExe -C $repo commit --quiet -m baseline
    if ($LASTEXITCODE -ne 0) { throw "Could not create fixture repository: $repo" }
    return $repo
}

function Invoke-SetupProcess {
    param(
        [Parameter(Mandatory)] [string]$ScriptPath,
        [Parameter(Mandatory)] [string]$RepoPath,
        [Parameter(Mandatory)] [string]$InstallDir,
        [Parameter(Mandatory)] [string[]]$Arguments,
        [string]$PathOverride
    )

    # Windows PowerShell 5.1 surfaces expected native stderr as ErrorRecord.
    $ErrorActionPreference = 'Continue'
    $oldPath = $env:PATH
    try {
        if ($PSBoundParameters.ContainsKey('PathOverride')) {
            $env:PATH = $PathOverride
        }
        $output = @(& $Script:PowerShellExe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $ScriptPath `
                -RepoPath $RepoPath -InstallDir $InstallDir @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $env:PATH = $oldPath
    }

    return [PSCustomObject]@{
        ExitCode = $exitCode
        Output   = $output
    }
}

function Get-FilterValue {
    param(
        [Parameter(Mandatory)] [string]$Repo,
        [Parameter(Mandatory)] [string]$Name
    )

    $value = & $Script:GitExe -C $Repo config --get $Name 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return $value
}

function Assert-FilterRollback {
    param(
        [Parameter(Mandatory)] [string]$Repo,
        [Parameter(Mandatory)] [string]$ConfigPath,
        [Parameter(Mandatory)] [byte[]]$OriginalBytes,
        [Parameter(Mandatory)] [string]$FilterName,
        [Parameter(Mandatory)] [string]$Label,
        [Parameter(Mandatory)]$Result
    )

    Assert-True -Condition ($Result.ExitCode -ne 0) -Label "$Label exits non-zero"
    Assert-BytesEqual -Actual ([IO.File]::ReadAllBytes($ConfigPath)) -Expected $OriginalBytes -Label "$Label preserves config bytes"
    Assert-True -Condition (-not (Test-Path "$ConfigPath.bak")) -Label "$Label leaves no backup"
    Assert-True -Condition (-not (Get-FilterValue -Repo $Repo -Name "filter.$FilterName.clean")) -Label "$Label removes git config"

    $attributesPath = Join-Path $Repo '.git/info/attributes'
    Assert-True -Condition (-not (Test-Path $attributesPath)) -Label "$Label restores absent attributes file"
    Assert-True -Condition (-not (Test-Path (Join-Path $Repo ".git/$FilterName"))) -Label "$Label removes filter directory"
    Assert-True -Condition (($Result.Output -join "`n") -match 'runtime probe failed') -Label "$Label reports runtime failure"
    Assert-True -Condition (($Result.Output -join "`n") -match 'Rollback result: removed') -Label "$Label reports rollback"
}

$scriptPath = Join-Path $PSScriptRoot 'setup-xray.ps1'
if (-not (Test-Path $scriptPath)) { throw "setup-xray.ps1 not found: $scriptPath" }
$Script:GitExe = (Get-Command git -CommandType Application -ErrorAction Stop).Source
$Script:PowerShellExe = (Get-Process -Id $PID).Path
$minimalPath = $PSHOME
$root = New-TempDirectory
$oldGlobalConfig = $env:GIT_CONFIG_GLOBAL
$oldNoSystemConfig = $env:GIT_CONFIG_NOSYSTEM

try {
    $isolatedGlobalConfig = Join-Path $root 'global.gitconfig'
    New-Item -ItemType File -Path $isolatedGlobalConfig -Force | Out-Null
    $env:GIT_CONFIG_GLOBAL = $isolatedGlobalConfig
    $env:GIT_CONFIG_NOSYSTEM = '1'

    # Load the pure download helpers without executing setup's imperative body.
    $tokens = $null
    $parseErrors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$parseErrors)
    if (@($parseErrors).Count -gt 0) { throw "Parse errors in setup-xray.ps1: $($parseErrors -join '; ')" }
    foreach ($functionName in @('Test-WindowsExecutable', 'Save-XrayReleaseAsset')) {
        $functionAst = $ast.Find({
                param($node)
                $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $functionName
            }, $true)
        if (-not $functionAst) { throw "$functionName not found in setup-xray.ps1" }
        . ([scriptblock]::Create($functionAst.Extent.Text))
    }

    $script:DownloadMode = 'valid'
    $script:DownloadUri = $null
    function Invoke-WebRequest {
        [CmdletBinding()]
        param(
            [Parameter(Mandatory)]$Uri,
            [Parameter(Mandatory)] [string]$OutFile,
            [switch]$UseBasicParsing
        )

        $script:DownloadUri = "$Uri"
        if ($script:DownloadMode -eq 'throw') { throw 'simulated direct download failure' }
        if ($script:DownloadMode -eq 'invalid') {
            [IO.File]::WriteAllText($OutFile, '<html>not a binary</html>', [Text.UTF8Encoding]::new($false))
            return
        }
        Write-FakeWindowsExecutable -Path $OutFile
    }

    $downloadDir = Join-Path $root 'download-helper'
    New-Item -ItemType Directory -Path $downloadDir -Force | Out-Null
    $downloadPath = Join-Path $downloadDir 'xray.exe'
    $oldPath = $env:PATH
    try {
        $env:PATH = $minimalPath
        Save-XrayReleaseAsset -GithubRepo 'owner/repo' -Destination $downloadPath
    }
    finally { $env:PATH = $oldPath }
    Assert-Equal -Actual $script:DownloadUri -Expected 'https://github.com/owner/repo/releases/latest/download/xray.exe' -Label 'T1 direct download URL'
    Assert-True -Condition (Test-WindowsExecutable -Path $downloadPath) -Label 'T1 direct download accepts PE payload'

    $script:DownloadMode = 'invalid'
    $invalidMessage = $null
    try { Save-XrayReleaseAsset -GithubRepo 'owner/repo' -Destination $downloadPath }
    catch { $invalidMessage = $_.Exception.Message }
    Assert-True -Condition ($invalidMessage -match 'not a valid Windows executable') -Label 'T2 invalid payload is rejected'

    $script:DownloadMode = 'throw'
    $missingGhMessage = $null
    try {
        $env:PATH = $minimalPath
        Save-XrayReleaseAsset -GithubRepo 'owner/repo' -Destination $downloadPath
    }
    catch { $missingGhMessage = $_.Exception.Message }
    finally { $env:PATH = $oldPath }
    Assert-True -Condition ($missingGhMessage -match 'Download failed from https://github.com/owner/repo') -Label 'T3 missing gh reports direct failure'

    $invalidRepoMessage = $null
    try { Save-XrayReleaseAsset -GithubRepo '../invalid' -Destination $downloadPath }
    catch { $invalidRepoMessage = $_.Exception.Message }
    Assert-True -Condition ($invalidRepoMessage -match 'Invalid GitHub repository') -Label 'T3b malformed GitHub repo is rejected early'

    $fakeBin = Join-Path $root 'fake-bin'
    New-Item -ItemType Directory -Path $fakeBin -Force | Out-Null
    $fakeGh = @'
@echo off
set "TARGET="
:next
if "%~1"=="" goto done
if /I "%~1"=="--dir" (
  set "TARGET=%~2"
  shift
)
shift
goto next
:done
if not defined TARGET exit /b 2
copy /b "%TARGET%\..\fake-xray.exe" "%TARGET%\xray.exe" >nul
exit /b 0
'@
    Set-Content -Path (Join-Path $fakeBin 'gh.cmd') -Value $fakeGh -Encoding ASCII
    Write-FakeWindowsExecutable -Path (Join-Path $root 'fake-xray.exe')
    try {
        $env:PATH = "$fakeBin;$minimalPath"
        Save-XrayReleaseAsset -GithubRepo 'owner/private-repo' -Destination $downloadPath -WarningAction SilentlyContinue
    }
    finally { $env:PATH = $oldPath }
    Assert-True -Condition (Test-WindowsExecutable -Path $downloadPath) -Label 'T4 gh fallback accepts PE payload'

    # A true non-Git folder remains supported in Visible mode without git.
    $plainRepo = Join-Path $root 'plain-visible'
    New-Item -ItemType Directory -Path $plainRepo -Force | Out-Null
    $plainInstall = New-InstallDirectory -Root $root
    $plainResult = Invoke-SetupProcess -ScriptPath $scriptPath -RepoPath $plainRepo -InstallDir $plainInstall `
        -Arguments @('-SkipDownload', '-Extensions', 'ts', '-EnableVSCode', '-GitVisibility', 'Visible', '-Force') `
        -PathOverride $minimalPath
    Assert-Equal -Actual $plainResult.ExitCode -Expected 0 -Label 'T5 non-Git Visible succeeds without git'
    Assert-True -Condition (Test-Path (Join-Path $plainRepo '.vscode/mcp.json')) -Label 'T5 non-Git Visible writes config'

    $hiddenRepo = Join-Path $root 'plain-hidden'
    New-Item -ItemType Directory -Path $hiddenRepo -Force | Out-Null
    $hiddenInstall = New-InstallDirectory -Root $root
    $hiddenResult = Invoke-SetupProcess -ScriptPath $scriptPath -RepoPath $hiddenRepo -InstallDir $hiddenInstall `
        -Arguments @('-SkipDownload', '-Extensions', 'ts', '-EnableVSCode', '-GitVisibility', 'Hidden', '-Force') `
        -PathOverride $minimalPath
    Assert-True -Condition ($hiddenResult.ExitCode -ne 0) -Label 'T6 non-Git Hidden fails without git'
    Assert-True -Condition (-not (Test-Path (Join-Path $hiddenRepo '.vscode/mcp.json'))) -Label 'T6 non-Git Hidden writes nothing'

    $managedRepo = New-GitFixture -Root $root -Name 'managed-no-git'
    $managedInstall = New-InstallDirectory -Root $root
    $managedResult = Invoke-SetupProcess -ScriptPath $scriptPath -RepoPath $managedRepo -InstallDir $managedInstall `
        -Arguments @('-SkipDownload', '-Extensions', 'ts', '-EnableVSCode', '-GitVisibility', 'Visible', '-Force') `
        -PathOverride $minimalPath
    Assert-True -Condition ($managedResult.ExitCode -ne 0) -Label 'T7 Git metadata without git fails'
    Assert-True -Condition (-not (Test-Path (Join-Path $managedRepo '.vscode/mcp.json'))) -Label 'T7 Git metadata failure writes nothing'

    $shimDir = Join-Path $root 'broken-git-shim'
    New-Item -ItemType Directory -Path $shimDir -Force | Out-Null
    Set-Content -Path (Join-Path $shimDir 'git.cmd') -Value '@exit /b 128' -Encoding ASCII
    $brokenResult = Invoke-SetupProcess -ScriptPath $scriptPath -RepoPath $managedRepo -InstallDir $managedInstall `
        -Arguments @('-SkipDownload', '-Extensions', 'ts', '-EnableVSCode', '-GitVisibility', 'Visible', '-Force') `
        -PathOverride "$shimDir;$minimalPath"
    Assert-True -Condition ($brokenResult.ExitCode -ne 0) -Label 'T8 broken git fails closed'

    $restoreRepo = New-GitFixture -Root $root -Name 'restore-no-git' -Config 'VsCode'
    $restoreInstall = New-InstallDirectory -Root $root
    $restoreConfig = Join-Path $restoreRepo '.vscode/mcp.json'
    $restoreExpected = [IO.File]::ReadAllBytes($restoreConfig)
    Copy-Item -LiteralPath $restoreConfig -Destination "$restoreConfig.bak" -Force
    [IO.File]::WriteAllText($restoreConfig, "{`n  `"servers`": {`n    `"broken`": {}`n  }`n}`n", [Text.UTF8Encoding]::new($false))
    $restoreResult = Invoke-SetupProcess -ScriptPath $scriptPath -RepoPath $restoreRepo -InstallDir $restoreInstall `
        -Arguments @('-Restore') -PathOverride $minimalPath
    Assert-Equal -Actual $restoreResult.ExitCode -Expected 0 -Label 'T8b restore succeeds without git'
    Assert-BytesEqual -Actual ([IO.File]::ReadAllBytes($restoreConfig)) -Expected $restoreExpected -Label 'T8b restore recovers config bytes'
    Assert-True -Condition (-not (Test-Path "$restoreConfig.bak")) -Label 'T8b restore consumes backup'


    # Healthy runtime probe: no persistent object or probe directory.
    $healthyRepo = New-GitFixture -Root $root -Name 'healthy-filter' -Config 'VsCode'
    $healthyGitConfig = @(
        (& $Script:GitExe -C $healthyRepo config --get core.autocrlf),
        (& $Script:GitExe -C $healthyRepo config --get core.safecrlf))
    Assert-Equal -Actual ($healthyGitConfig -join ',') -Expected 'true,true' -Label 'T9 stock CRLF safety settings are active'
    $healthyInstall = New-InstallDirectory -Root $root
    $unreachableBefore = @(& $Script:GitExe -C $healthyRepo fsck --unreachable --no-reflogs 2>$null | Sort-Object)
    $healthyResult = Invoke-SetupProcess -ScriptPath $scriptPath -RepoPath $healthyRepo -InstallDir $healthyInstall `
        -Arguments @('-SkipDownload', '-Extensions', 'ts', '-EnableVSCode', '-GitVisibility', 'Hidden', '-Force')
    $unreachableAfter = @(& $Script:GitExe -C $healthyRepo fsck --unreachable --no-reflogs 2>$null | Sort-Object)
    Assert-Equal -Actual $healthyResult.ExitCode -Expected 0 -Label 'T9 healthy filter runtime succeeds'
    Assert-Equal -Actual ($unreachableAfter -join "`n") -Expected ($unreachableBefore -join "`n") -Label 'T9 runtime probe leaves no unreachable Git object'
    $runtimeProbeFiles = @(Get-ChildItem -Path (Join-Path $healthyRepo '.git/xray-vscode-mcp') -Filter '.runtime-probe-*' -ErrorAction SilentlyContinue)
    Assert-Equal -Actual $runtimeProbeFiles.Count -Expected 0 -Label 'T9 runtime probe cleans temporary files'

    $legacySuccessRepo = New-GitFixture -Root $root -Name 'legacy-success' -Config 'VsCode'
    $legacySuccessInstall = New-InstallDirectory -Root $root
    $legacySuccessConfig = Join-Path $legacySuccessRepo '.vscode/mcp.json'
    & $Script:GitExe -C $legacySuccessRepo update-index --skip-worktree -- '.vscode/mcp.json'
    if ($LASTEXITCODE -ne 0) { throw 'Could not create successful legacy skip-worktree fixture' }
    $legacySuccessHeadHash = & $Script:GitExe -C $legacySuccessRepo rev-parse 'HEAD:.vscode/mcp.json'
    $legacySuccessResult = Invoke-SetupProcess -ScriptPath $scriptPath -RepoPath $legacySuccessRepo -InstallDir $legacySuccessInstall `
        -Arguments @('-SkipDownload', '-Extensions', 'ts', '-EnableVSCode', '-GitVisibility', 'Hidden', '-Force')
    $legacySuccessFlags = & $Script:GitExe -C $legacySuccessRepo ls-files -v -- '.vscode/mcp.json'
    $legacySuccessStatus = @(& $Script:GitExe -C $legacySuccessRepo status --short -- '.vscode/mcp.json')
    $legacySuccessIndexHash = & $Script:GitExe -C $legacySuccessRepo rev-parse ':.vscode/mcp.json'
    Assert-Equal -Actual $legacySuccessResult.ExitCode -Expected 0 -Label 'T9b legacy skip-worktree upgrade succeeds'
    Assert-True -Condition ($legacySuccessFlags -match '^H ') -Label 'T9b successful upgrade clears skip-worktree'
    Assert-Equal -Actual $legacySuccessStatus.Count -Expected 0 -Label 'T9b successful upgrade keeps config clean'
    Assert-Equal -Actual $legacySuccessIndexHash -Expected $legacySuccessHeadHash -Label 'T9b successful upgrade keeps canonical index blob'
    Assert-True -Condition ([IO.File]::ReadAllText($legacySuccessConfig) -match '_xrayMcpMarker') -Label 'T9b successful upgrade injects working-tree marker'


    $configPath = Join-Path $healthyRepo '.vscode/mcp.json'
    $configBeforeUninstall = [IO.File]::ReadAllBytes($configPath)
    $filterBeforeUninstall = Get-FilterValue -Repo $healthyRepo -Name 'filter.xray-vscode-mcp.clean'
    $uninstallResult = Invoke-SetupProcess -ScriptPath $scriptPath -RepoPath $healthyRepo -InstallDir $healthyInstall `
        -Arguments @('-Uninstall', '-KeepBinary') -PathOverride $minimalPath
    Assert-True -Condition ($uninstallResult.ExitCode -ne 0) -Label 'T10 uninstall without git fails closed'
    Assert-BytesEqual -Actual ([IO.File]::ReadAllBytes($configPath)) -Expected $configBeforeUninstall -Label 'T10 uninstall preserves config'
    Assert-Equal -Actual (Get-FilterValue -Repo $healthyRepo -Name 'filter.xray-vscode-mcp.clean') -Expected $filterBeforeUninstall -Label 'T10 uninstall preserves filter'
    Assert-True -Condition (Test-Path (Join-Path $healthyInstall 'xray.exe')) -Label 'T10 uninstall preserves binary'

    $sourceText = [IO.File]::ReadAllText($scriptPath)
    $missingBashScript = Join-Path $root 'setup-missing-bash.ps1'
    $missingBashText = [Regex]::Replace(
        $sourceText,
        '(?m)^(\s*\$(?:smudgeCmd|cleanCmd)\s*=\s*)''bash',
        { param($match) $match.Groups[1].Value + "'xray-missing-bash" })
    [IO.File]::WriteAllText($missingBashScript, $missingBashText, [Text.UTF8Encoding]::new($false))

    $missingBashRepo = New-GitFixture -Root $root -Name 'missing-bash' -Config 'VsCode'
    $missingBashInstall = New-InstallDirectory -Root $root
    $missingBashConfig = Join-Path $missingBashRepo '.vscode/mcp.json'
    $legacyConfig = "{`r`n  `"servers`": {`r`n    `"xray`": { `"type`": `"stdio`", `"command`": `"legacy-xray.exe`", `"args`": [] }`r`n  }`r`n}`r`n"
    [IO.File]::WriteAllText($missingBashConfig, $legacyConfig, [Text.UTF8Encoding]::new($false))
    & $Script:GitExe -C $missingBashRepo update-index --skip-worktree -- '.vscode/mcp.json'
    if ($LASTEXITCODE -ne 0) { throw 'Could not create legacy skip-worktree fixture' }
    $missingBashOriginal = [IO.File]::ReadAllBytes($missingBashConfig)
    $missingBashResult = Invoke-SetupProcess -ScriptPath $missingBashScript -RepoPath $missingBashRepo -InstallDir $missingBashInstall `
        -Arguments @('-SkipDownload', '-Extensions', 'ts', '-EnableVSCode', '-GitVisibility', 'Hidden', '-Force')
    Assert-FilterRollback -Repo $missingBashRepo -ConfigPath $missingBashConfig -OriginalBytes $missingBashOriginal `
        -FilterName 'xray-vscode-mcp' -Label 'T11 missing bash' -Result $missingBashResult
    $missingBashFlags = & $Script:GitExe -C $missingBashRepo ls-files -v -- '.vscode/mcp.json'
    $missingBashStatus = @(& $Script:GitExe -C $missingBashRepo status --short -- '.vscode/mcp.json')
    Assert-True -Condition ($missingBashFlags -match '^S ') -Label 'T11 missing bash preserves legacy skip-worktree'
    Assert-Equal -Actual $missingBashStatus.Count -Expected 0 -Label 'T11 missing bash keeps legacy config hidden and clean'

    $missingPerlScript = Join-Path $root 'setup-missing-perl.ps1'
    $missingPerlText = $sourceText.Replace('exec perl -e', 'exec xray-missing-perl -e')
    [IO.File]::WriteAllText($missingPerlScript, $missingPerlText, [Text.UTF8Encoding]::new($false))

    $missingPerlRepo = New-GitFixture -Root $root -Name 'missing-perl' -Config 'CopilotCli'
    $missingPerlInstall = New-InstallDirectory -Root $root
    $missingPerlConfig = Join-Path $missingPerlRepo '.mcp.json'
    $missingPerlOriginal = [IO.File]::ReadAllBytes($missingPerlConfig)
    $missingPerlResult = Invoke-SetupProcess -ScriptPath $missingPerlScript -RepoPath $missingPerlRepo -InstallDir $missingPerlInstall `
        -Arguments @('-SkipDownload', '-Extensions', 'ts', '-EnableCopilotCli', '-GitVisibility', 'Hidden', '-Force')
    Assert-FilterRollback -Repo $missingPerlRepo -ConfigPath $missingPerlConfig -OriginalBytes $missingPerlOriginal `
        -FilterName 'xray-mcp' -Label 'T12 missing perl' -Result $missingPerlResult
    $backupFailureScript = Join-Path $root 'setup-backup-failure.ps1'
    $backupFailureText = $sourceText.Replace(
        '    Copy-Item -Path $Path -Destination $bakPath -Force',
        "    throw 'simulated backup failure'")
    [IO.File]::WriteAllText($backupFailureScript, $backupFailureText, [Text.UTF8Encoding]::new($false))

    $backupFailureRepo = New-GitFixture -Root $root -Name 'backup-failure' -Config 'VsCode'
    $backupFailureInstall = New-InstallDirectory -Root $root
    $backupFailureConfig = Join-Path $backupFailureRepo '.vscode/mcp.json'
    $backupFailureOriginal = [IO.File]::ReadAllBytes($backupFailureConfig)
    $backupFailureResult = Invoke-SetupProcess -ScriptPath $backupFailureScript -RepoPath $backupFailureRepo -InstallDir $backupFailureInstall `
        -Arguments @('-SkipDownload', '-Extensions', 'ts', '-EnableVSCode', '-GitVisibility', 'Hidden', '-Force')
    Assert-True -Condition ($backupFailureResult.ExitCode -ne 0) -Label 'T13 backup failure exits non-zero'
    Assert-BytesEqual -Actual ([IO.File]::ReadAllBytes($backupFailureConfig)) -Expected $backupFailureOriginal -Label 'T13 backup failure preserves config bytes'
    Assert-True -Condition (-not (Test-Path "$backupFailureConfig.bak")) -Label 'T13 backup failure leaves no backup'
    Assert-True -Condition (-not (Get-FilterValue -Repo $backupFailureRepo -Name 'filter.xray-vscode-mcp.clean')) -Label 'T13 backup failure removes git config'
    $backupFailureAttributesPath = Join-Path $backupFailureRepo '.git/info/attributes'
    Assert-True -Condition (-not (Test-Path $backupFailureAttributesPath)) -Label 'T13 backup failure restores absent attributes file'
    Assert-True -Condition (-not (Test-Path (Join-Path $backupFailureRepo '.git/xray-vscode-mcp'))) -Label 'T13 backup failure removes filter directory'
    Assert-True -Condition (($backupFailureResult.Output -join "`n") -match 'Could not create MCP config backup') -Label 'T13 backup failure reports cause'
    Assert-True -Condition (($backupFailureResult.Output -join "`n") -match 'Filter rollback after backup failure: removed') -Label 'T13 backup failure reports rollback'

    $updateIndexFailureScript = Join-Path $root 'setup-update-index-failure.ps1'
    $updateIndexNeedle = '            $gitOutput = & $gitCommand.Source update-index --no-skip-worktree -- $RelativePath 2>&1'
    $updateIndexReplacement = '            $gitOutput = & $gitCommand.Source update-index --xray-force-failure -- $RelativePath 2>&1'
    $updateIndexFailureText = $sourceText.Replace($updateIndexNeedle, $updateIndexReplacement)
    if ($updateIndexFailureText -eq $sourceText) { throw 'Could not create update-index failure mutation' }
    [IO.File]::WriteAllText($updateIndexFailureScript, $updateIndexFailureText, [Text.UTF8Encoding]::new($false))

    $updateIndexFailureRepo = New-GitFixture -Root $root -Name 'update-index-failure' -Config 'VsCode'
    $updateIndexFailureInstall = New-InstallDirectory -Root $root
    $updateIndexFailureConfig = Join-Path $updateIndexFailureRepo '.vscode/mcp.json'
    $updateIndexFailureOriginal = [IO.File]::ReadAllBytes($updateIndexFailureConfig)
    & $Script:GitExe -C $updateIndexFailureRepo update-index --skip-worktree -- '.vscode/mcp.json'
    if ($LASTEXITCODE -ne 0) { throw 'Could not create update-index failure skip-worktree fixture' }
    $updateIndexFailureResult = Invoke-SetupProcess -ScriptPath $updateIndexFailureScript -RepoPath $updateIndexFailureRepo -InstallDir $updateIndexFailureInstall `
        -Arguments @('-SkipDownload', '-Extensions', 'ts', '-EnableVSCode', '-GitVisibility', 'Hidden', '-Force')
    $updateIndexFailureFlags = & $Script:GitExe -C $updateIndexFailureRepo ls-files -v -- '.vscode/mcp.json'
    $updateIndexFailureStatus = @(& $Script:GitExe -C $updateIndexFailureRepo status --short -- '.vscode/mcp.json')
    Assert-True -Condition ($updateIndexFailureResult.ExitCode -ne 0) -Label 'T14 update-index failure exits non-zero'
    Assert-BytesEqual -Actual ([IO.File]::ReadAllBytes($updateIndexFailureConfig)) -Expected $updateIndexFailureOriginal -Label 'T14 update-index failure restores config bytes'
    Assert-True -Condition (-not (Test-Path "$updateIndexFailureConfig.bak")) -Label 'T14 update-index failure consumes backup'
    Assert-True -Condition (-not (Get-FilterValue -Repo $updateIndexFailureRepo -Name 'filter.xray-vscode-mcp.clean')) -Label 'T14 update-index failure removes git config'
    Assert-True -Condition (-not (Test-Path (Join-Path $updateIndexFailureRepo '.git/info/attributes'))) -Label 'T14 update-index failure removes attributes'
    Assert-True -Condition (-not (Test-Path (Join-Path $updateIndexFailureRepo '.git/xray-vscode-mcp'))) -Label 'T14 update-index failure removes filter directory'
    Assert-True -Condition ($updateIndexFailureFlags -match '^S ') -Label 'T14 update-index failure leaves skip-worktree untouched'
    Assert-Equal -Actual $updateIndexFailureStatus.Count -Expected 0 -Label 'T14 update-index failure keeps config clean'
    Assert-True -Condition (($updateIndexFailureResult.Output -join "`n") -match 'Could not clear skip-worktree') -Label 'T14 update-index failure reports cause'
    Assert-True -Condition (($updateIndexFailureResult.Output -join "`n") -match 'Filter rollback after update-index failure: removed') -Label 'T14 update-index failure reports rollback'


}
finally {
    $env:GIT_CONFIG_GLOBAL = $oldGlobalConfig
    $env:GIT_CONFIG_NOSYSTEM = $oldNoSystemConfig
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}

$expectedAssertions = 65
if (($Script:Passed + $Script:Failed) -ne $expectedAssertions) {
    $Script:Failed++
    Write-Host "FAIL  expected $expectedAssertions assertions, ran $($Script:Passed + $Script:Failed - 1)" -ForegroundColor Red
}

Write-Host "`n=== Summary ===" -ForegroundColor Cyan
Write-Host "Passed: $Script:Passed" -ForegroundColor Green
Write-Host "Failed: $Script:Failed" -ForegroundColor $(if ($Script:Failed -eq 0) { 'Green' } else { 'Red' })
if ($Script:Failed -gt 0) { exit 1 }
