param(
    [switch]$CheckOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$artifactRoot = (Resolve-Path -LiteralPath $PSScriptRoot).Path
$workspaceRoot = (Resolve-Path -LiteralPath (Join-Path $artifactRoot '..\..')).Path
$originalRoot = Join-Path $artifactRoot 'original'
$changedFilesPath = Join-Path $artifactRoot 'changed-files.txt'
$baselineManifestPath = Join-Path $artifactRoot 'baseline.sha256'
$modifiedManifestPath = Join-Path $artifactRoot 'modified-snapshot.sha256'

function Read-HashManifest([string]$Path) {
    $entries = [ordered]@{}
    foreach ($line in Get-Content -LiteralPath $Path) {
        if ($line -notmatch '^([0-9a-f]{64})  (.+)$') {
            throw "Invalid hash manifest line in ${Path}: $line"
        }
        $entries[$Matches[2]] = $Matches[1]
    }
    return $entries
}

function Resolve-WorkspaceTarget([string]$RelativePath) {
    if ([System.IO.Path]::IsPathRooted($RelativePath) -or $RelativePath -match '(^|[\\/])\.\.([\\/]|$)') {
        throw "Unsafe relative path: $RelativePath"
    }

    $target = [System.IO.Path]::GetFullPath((Join-Path $workspaceRoot $RelativePath))
    $workspacePrefix = $workspaceRoot.TrimEnd('\') + '\'
    if (-not $target.StartsWith($workspacePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Rollback target leaves workspace: $target"
    }
    return $target
}

function Assert-Hash([string]$Path, [string]$ExpectedHash, [string]$Role) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Role file is missing: $Path"
    }
    $actualHash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $ExpectedHash) {
        throw "$Role hash mismatch: $Path (expected $ExpectedHash, actual $actualHash)"
    }
}

$baseline = Read-HashManifest $baselineManifestPath
$modified = Read-HashManifest $modifiedManifestPath
$changedFiles = @(Get-Content -LiteralPath $changedFilesPath | Where-Object { $_.Length -gt 0 })

foreach ($entry in $baseline.GetEnumerator()) {
    Assert-Hash (Join-Path $originalRoot $entry.Key) $entry.Value 'Original snapshot'
}

foreach ($relativePath in $changedFiles) {
    if (-not $modified.Contains($relativePath)) {
        throw "Changed file is missing from modified manifest: $relativePath"
    }
    Assert-Hash (Resolve-WorkspaceTarget $relativePath) $modified[$relativePath] 'Modified workspace'
}

if ($CheckOnly) {
    Write-Output "PASS: rollback inputs verified ($($changedFiles.Count) changed files, $($baseline.Count) original snapshots)."
    Write-Output 'PASS: no workspace files were changed because -CheckOnly was supplied.'
    exit 0
}

foreach ($relativePath in $changedFiles) {
    $target = Resolve-WorkspaceTarget $relativePath
    $original = Join-Path $originalRoot $relativePath
    if (Test-Path -LiteralPath $original -PathType Leaf) {
        $parent = Split-Path -Parent $target
        if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
            New-Item -ItemType Directory -Path $parent | Out-Null
        }
        Copy-Item -LiteralPath $original -Destination $target -Force
    } else {
        Remove-Item -LiteralPath $target -Force
    }
}

foreach ($relativePath in $changedFiles) {
    $target = Resolve-WorkspaceTarget $relativePath
    $original = Join-Path $originalRoot $relativePath
    if (Test-Path -LiteralPath $original -PathType Leaf) {
        Assert-Hash $target $baseline[$relativePath] 'Rolled-back workspace'
    } elseif (Test-Path -LiteralPath $target) {
        throw "Task-created file remains after rollback: $target"
    }
}

Write-Output "PASS: rollback restored the original task scope ($($changedFiles.Count) files checked)."
