[CmdletBinding()]
param(
    [ValidateSet('Preflight', 'Build')]
    [string]$Mode = 'Preflight',
    [string]$Repository = 'D:\NTE_DPS_TOOL',
    [string]$VerificationSource = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ExpectedHead = '86d21c622910f0007bb3ef9d02a98574d39708c0'
$ExpectedBranch = 'codex/review-priority-optimizations'
$FixtureRoot = Join-Path $Repository '.codex-fixtures\followup-2026-08-20'
$Deliverables = Join-Path $FixtureRoot 'deliverables'
$WorkRoot = Join-Path $FixtureRoot 'packaging-work'
$DiffFile = Join-Path $Deliverables 'DIFF_FILE'
$ModifiedFile = Join-Path $Deliverables 'MODIFIED_FILE'
$VerificationFile = Join-Path $Deliverables 'VERIFICATION.txt'
$RollbackFile = Join-Path $Deliverables 'ROLLBACK.sh'
$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Invoke-GitText {
    param([Parameter(Mandatory)][string[]]$Arguments)

    $output = @(& git @Arguments 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "git $($Arguments -join ' ') failed ($LASTEXITCODE):`n$($output -join "`n")"
    }
    return $output
}

function Write-GitStdoutRaw {
    param(
        [Parameter(Mandatory)][string[]]$Arguments,
        [Parameter(Mandatory)][string]$Destination
    )

    $git = (Get-Command git.exe -ErrorAction Stop).Source
    $processInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $processInfo.FileName = $git
    $processInfo.WorkingDirectory = $Repository
    $processInfo.UseShellExecute = $false
    $processInfo.RedirectStandardOutput = $true
    $processInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) {
        [void]$processInfo.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $processInfo
    if (-not $process.Start()) {
        throw "failed to start git $($Arguments -join ' ')"
    }

    $stderrTask = $process.StandardError.ReadToEndAsync()
    $stream = [System.IO.File]::Open(
        $Destination,
        [System.IO.FileMode]::Create,
        [System.IO.FileAccess]::Write,
        [System.IO.FileShare]::None
    )
    try {
        $process.StandardOutput.BaseStream.CopyTo($stream)
    }
    finally {
        $stream.Dispose()
    }
    $process.WaitForExit()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    if ($process.ExitCode -ne 0) {
        throw "git $($Arguments -join ' ') failed ($($process.ExitCode)):`n$stderr"
    }
}

function Set-AlternateIndex {
    param([Parameter(Mandatory)][string]$Path)
    $env:GIT_INDEX_FILE = $Path
}

function Write-RollbackScript {
    $content = @'
#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
patch_file="$script_dir/DIFF_FILE"
target="${1:?usage: ROLLBACK.sh <repository-copy>}"
expected_head="86d21c622910f0007bb3ef9d02a98574d39708c0"

test -s "$patch_file"
git -C "$target" rev-parse --is-inside-work-tree >/dev/null
actual_head="$(git -C "$target" rev-parse HEAD)"
test "$actual_head" = "$expected_head"

git -C "$target" apply --reverse --check "$patch_file"
git -C "$target" apply --reverse "$patch_file"

test -z "$(git -C "$target" status --porcelain=v1 --untracked-files=all)"
test "$(git -C "$target" rev-parse HEAD)" = "$expected_head"

echo "ROLLBACK_OK head=$actual_head status=clean restored=baseline"
'@
    [System.IO.File]::WriteAllText($RollbackFile, $content.Replace("`r`n", "`n"), $Utf8NoBom)
}

function Assert-RollbackScript {
    $gitBash = 'C:\Program Files\Git\bin\bash.exe'
    if (-not (Test-Path -LiteralPath $gitBash -PathType Leaf)) {
        $gitBash = (Get-Command bash.exe -ErrorAction Stop).Source
    }
    & $gitBash -n $RollbackFile
    if ($LASTEXITCODE -ne 0) {
        throw "ROLLBACK.sh syntax check failed ($LASTEXITCODE)"
    }
    & $gitBash -c 'chmod +x "$1" && test -x "$1"' _ $RollbackFile
    if ($LASTEXITCODE -ne 0) {
        throw "ROLLBACK.sh executable check failed ($LASTEXITCODE)"
    }
}

function New-TaskIndex {
    param([Parameter(Mandatory)][string]$IndexPath)

    if (Test-Path -LiteralPath $IndexPath) {
        Remove-Item -LiteralPath $IndexPath -Force
    }
    Set-AlternateIndex $IndexPath
    [void](Invoke-GitText @('read-tree', 'HEAD'))
    [void](Invoke-GitText @(
        'add', '-A', '--', '.',
        ':(exclude,top,glob).codex-fixtures/**',
        ':(exclude,glob)**/target/**',
        ':(exclude,glob)**/dist/**',
        ':(exclude,glob)**/node_modules/**'
    ))
}

function Get-StagedPaths {
    param([Parameter(Mandatory)][string[]]$DiffFilter)

    $arguments = @(
        '-c', 'core.quotePath=false',
        'diff', '--cached', '--no-renames', '--name-only'
    )
    foreach ($filter in $DiffFilter) {
        $arguments += "--diff-filter=$filter"
    }
    $arguments += 'HEAD'
    return @((Invoke-GitText $arguments) | Where-Object { $_ -ne '' })
}

function Assert-NoExcludedPaths {
    param([Parameter(Mandatory)][string[]]$Paths)

    $excluded = @($Paths | Where-Object {
        $_ -match '(^|/)(\.codex-fixtures|target|dist|node_modules)(/|$)'
    })
    if ($excluded.Count -ne 0) {
        throw "excluded paths entered task index:`n$($excluded -join "`n")"
    }
}

function Test-PatchAgainstIndexes {
    param(
        [Parameter(Mandatory)][string]$Patch,
        [Parameter(Mandatory)][string]$ModifiedIndex,
        [Parameter(Mandatory)][string]$CheckIndex
    )

    Set-AlternateIndex $ModifiedIndex
    $modifiedTree = (Invoke-GitText @('write-tree') | Select-Object -Last 1).Trim()
    $headTree = (Invoke-GitText @('rev-parse', 'HEAD^{tree}') | Select-Object -Last 1).Trim()

    if (Test-Path -LiteralPath $CheckIndex) {
        Remove-Item -LiteralPath $CheckIndex -Force
    }
    Set-AlternateIndex $CheckIndex
    [void](Invoke-GitText @('read-tree', 'HEAD'))
    [void](Invoke-GitText @('apply', '--cached', '--check', $Patch))
    [void](Invoke-GitText @('apply', '--cached', $Patch))
    $appliedTree = (Invoke-GitText @('write-tree') | Select-Object -Last 1).Trim()
    if ($appliedTree -ne $modifiedTree) {
        throw "forward patch tree mismatch: expected=$modifiedTree actual=$appliedTree"
    }

    [void](Invoke-GitText @('apply', '--cached', '--reverse', '--check', $Patch))
    [void](Invoke-GitText @('apply', '--cached', '--reverse', $Patch))
    $restoredTree = (Invoke-GitText @('write-tree') | Select-Object -Last 1).Trim()
    if ($restoredTree -ne $headTree) {
        throw "reverse patch tree mismatch: expected=$headTree actual=$restoredTree"
    }

    return [pscustomobject]@{
        HeadTree = $headTree
        ModifiedTree = $modifiedTree
        RestoredTree = $restoredTree
    }
}

function Write-ModifiedArchive {
    param(
        [Parameter(Mandatory)][string]$IndexPath,
        [Parameter(Mandatory)][string[]]$PresentPaths,
        [Parameter(Mandatory)][string[]]$DeletedPaths
    )

    $stageRoot = Join-Path $WorkRoot 'archive-root'
    if (Test-Path -LiteralPath $stageRoot) {
        Remove-Item -LiteralPath $stageRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path $stageRoot -Force | Out-Null

    Set-AlternateIndex $IndexPath
    $checkoutPrefix = $stageRoot.Replace('\', '/') + '/'
    foreach ($path in $PresentPaths) {
        [void](Invoke-GitText @('checkout-index', '--force', "--prefix=$checkoutPrefix", '--', $path))
    }

    $statusLines = @(Invoke-GitText @(
        '-c', 'core.quotePath=false',
        'diff', '--cached', '--no-renames', '--name-status', 'HEAD'
    ))
    $manifest = [System.Collections.Generic.List[string]]::new()
    $manifest.Add("BASE_HEAD=$ExpectedHead")
    $manifest.Add("BRANCH=$ExpectedBranch")
    $manifest.Add("PRESENT_FILE_COUNT=$($PresentPaths.Count)")
    $manifest.Add("DELETED_FILE_COUNT=$($DeletedPaths.Count)")
    $manifest.Add('')
    $manifest.Add('STATUS<TAB>PATH')
    foreach ($line in $statusLines) {
        $manifest.Add($line)
    }
    $manifest.Add('')
    $manifest.Add('SHA256<TAB>PATH')
    foreach ($path in $PresentPaths) {
        $materialized = Join-Path $stageRoot ($path.Replace('/', '\'))
        if (-not (Test-Path -LiteralPath $materialized -PathType Leaf)) {
            throw "staged file was not materialized: $path"
        }
        $sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $materialized).Hash.ToLowerInvariant()
        $manifest.Add("$sha256`t$path")
    }

    [System.IO.File]::WriteAllLines(
        (Join-Path $stageRoot 'MANIFEST.txt'),
        $manifest,
        $Utf8NoBom
    )
    [System.IO.File]::WriteAllText(
        (Join-Path $stageRoot 'BASE_HEAD.txt'),
        "$ExpectedHead`n",
        $Utf8NoBom
    )
    [System.IO.File]::WriteAllText(
        (Join-Path $stageRoot 'BRANCH.txt'),
        "$ExpectedBranch`n",
        $Utf8NoBom
    )
    [System.IO.File]::WriteAllLines(
        (Join-Path $stageRoot 'DELETED_FILES.txt'),
        $DeletedPaths,
        $Utf8NoBom
    )

    $originalHashes = [System.Collections.Generic.List[string]]::new()
    $originalHashes.Add("BASE_HEAD=$ExpectedHead")
    $originalHashes.Add('GIT_BLOB<TAB>PATH')
    foreach ($path in @($PresentPaths + $DeletedPaths | Sort-Object -Unique)) {
        & git cat-file -e "HEAD:$path" 2>$null
        if ($LASTEXITCODE -eq 0) {
            $blob = (Invoke-GitText @('rev-parse', "HEAD:$path") | Select-Object -Last 1).Trim()
            $originalHashes.Add("$blob`t$path")
        }
    }
    [System.IO.File]::WriteAllLines(
        (Join-Path $stageRoot 'ORIGINAL_HASHES.txt'),
        $originalHashes,
        $Utf8NoBom
    )

    $temporaryArchive = "$ModifiedFile.tmp"
    if (Test-Path -LiteralPath $temporaryArchive) {
        Remove-Item -LiteralPath $temporaryArchive -Force
    }
    & tar.exe -czf $temporaryArchive -C $stageRoot .
    if ($LASTEXITCODE -ne 0) {
        throw "tar creation failed ($LASTEXITCODE)"
    }
    $archiveListing = @(& tar.exe -tzf $temporaryArchive 2>&1)
    if ($LASTEXITCODE -ne 0 -or $archiveListing.Count -eq 0) {
        throw "tar reopen failed ($LASTEXITCODE): $($archiveListing -join "`n")"
    }
    Move-Item -LiteralPath $temporaryArchive -Destination $ModifiedFile -Force
    return $archiveListing
}

$originalIndex = [Environment]::GetEnvironmentVariable('GIT_INDEX_FILE', 'Process')
$taskIndex = Join-Path $WorkRoot "task-index.$PID"
$checkIndex = Join-Path $WorkRoot "check-index.$PID"
$previewDiff = Join-Path $WorkRoot 'PREVIEW_DIFF_FILE'

try {
    Set-Location -LiteralPath $Repository
    New-Item -ItemType Directory -Path $Deliverables -Force | Out-Null
    New-Item -ItemType Directory -Path $WorkRoot -Force | Out-Null

    $actualHead = (Invoke-GitText @('rev-parse', 'HEAD') | Select-Object -Last 1).Trim()
    $actualBranch = (Invoke-GitText @('branch', '--show-current') | Select-Object -Last 1).Trim()
    if ($actualHead -ne $ExpectedHead) {
        throw "unexpected HEAD: expected=$ExpectedHead actual=$actualHead"
    }
    if ($actualBranch -ne $ExpectedBranch) {
        throw "unexpected branch: expected=$ExpectedBranch actual=$actualBranch"
    }

    Write-RollbackScript
    Assert-RollbackScript

    New-TaskIndex -IndexPath $taskIndex
    $allPaths = Get-StagedPaths -DiffFilter @('ACMRTUXBD')
    $presentPaths = Get-StagedPaths -DiffFilter @('ACMRTUXB')
    $deletedPaths = Get-StagedPaths -DiffFilter @('D')
    if ($allPaths.Count -eq 0) {
        throw 'task index is empty'
    }
    Assert-NoExcludedPaths -Paths $allPaths

    Set-AlternateIndex $taskIndex
    $targetPatch = if ($Mode -eq 'Build') { $DiffFile } else { $previewDiff }
    Write-GitStdoutRaw -Arguments @(
        'diff', '--cached', '--binary', '--full-index', '--no-ext-diff', '--no-textconv', '--no-renames', 'HEAD'
    ) -Destination $targetPatch
    if ((Get-Item -LiteralPath $targetPatch).Length -eq 0) {
        throw 'generated patch is empty'
    }
    $treeCheck = Test-PatchAgainstIndexes -Patch $targetPatch -ModifiedIndex $taskIndex -CheckIndex $checkIndex

    $archiveListing = @()
    if ($Mode -eq 'Build') {
        if ($VerificationSource -ne '') {
            $resolvedVerification = (Resolve-Path -LiteralPath $VerificationSource).Path
            Copy-Item -LiteralPath $resolvedVerification -Destination $VerificationFile -Force
        }
        if (-not (Test-Path -LiteralPath $VerificationFile -PathType Leaf)) {
            throw "VERIFICATION.txt is required before Build: $VerificationFile"
        }
        if ((Get-Item -LiteralPath $VerificationFile).Length -eq 0) {
            throw 'VERIFICATION.txt is empty'
        }
        $verificationText = Get-Content -LiteralPath $VerificationFile -Raw
        foreach ($required in @('BASELINE', 'MODIFIED', 'ROLLBACK', $ExpectedBranch, 'DIFF_FILE', 'MODIFIED_FILE', 'VERIFICATION.txt', 'ROLLBACK.sh')) {
            if (-not $verificationText.Contains($required)) {
                throw "VERIFICATION.txt lacks required marker: $required"
            }
        }

        $archiveListing = @(Write-ModifiedArchive `
            -IndexPath $taskIndex `
            -PresentPaths $presentPaths `
            -DeletedPaths $deletedPaths)

        foreach ($artifact in @($ModifiedFile, $DiffFile, $VerificationFile, $RollbackFile)) {
            if (-not (Test-Path -LiteralPath $artifact -PathType Leaf)) {
                throw "missing deliverable: $artifact"
            }
            if ((Get-Item -LiteralPath $artifact).Length -eq 0) {
                throw "empty deliverable: $artifact"
            }
        }
    }

    $result = [System.Collections.Generic.List[string]]::new()
    $result.Add("MODE=$Mode")
    $result.Add("HEAD=$actualHead")
    $result.Add("BRANCH=$actualBranch")
    $result.Add("CHANGED_PATHS=$($allPaths.Count)")
    $result.Add("PRESENT_PATHS=$($presentPaths.Count)")
    $result.Add("DELETED_PATHS=$($deletedPaths.Count)")
    $result.Add("PATCH_BYTES=$((Get-Item -LiteralPath $targetPatch).Length)")
    $result.Add("HEAD_TREE=$($treeCheck.HeadTree)")
    $result.Add("MODIFIED_TREE=$($treeCheck.ModifiedTree)")
    $result.Add("RESTORED_TREE=$($treeCheck.RestoredTree)")
    $result.Add('FORWARD_INDEX_APPLY=PASS')
    $result.Add('REVERSE_INDEX_APPLY=PASS')
    $result.Add('EXCLUDED_PATHS=PASS')
    $result.Add('ROLLBACK_SYNTAX=PASS')
    $result.Add('ROLLBACK_EXECUTABLE=PASS')
    if ($Mode -eq 'Build') {
        $result.Add("ARCHIVE_ENTRIES=$($archiveListing.Count)")
        foreach ($artifact in @($ModifiedFile, $DiffFile, $VerificationFile, $RollbackFile)) {
            $item = Get-Item -LiteralPath $artifact
            $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $artifact).Hash.ToLowerInvariant()
            $result.Add("ARTIFACT=$($item.FullName)|BYTES=$($item.Length)|SHA256=$hash")
        }
    }

    $resultFile = Join-Path $WorkRoot $(if ($Mode -eq 'Build') { 'BUILD_RESULT.txt' } else { 'PREPARE_SELF_CHECK.txt' })
    [System.IO.File]::WriteAllLines($resultFile, $result, $Utf8NoBom)
    $result | ForEach-Object { Write-Host $_ }
    Write-Host "RESULT_FILE=$resultFile"
}
finally {
    if ($null -eq $originalIndex) {
        Remove-Item Env:GIT_INDEX_FILE -ErrorAction SilentlyContinue
    }
    else {
        $env:GIT_INDEX_FILE = $originalIndex
    }
    foreach ($temporaryIndex in @($taskIndex, $checkIndex)) {
        if (Test-Path -LiteralPath $temporaryIndex) {
            Remove-Item -LiteralPath $temporaryIndex -Force
        }
        $lockFile = "$temporaryIndex.lock"
        if (Test-Path -LiteralPath $lockFile) {
            Remove-Item -LiteralPath $lockFile -Force
        }
    }
}
