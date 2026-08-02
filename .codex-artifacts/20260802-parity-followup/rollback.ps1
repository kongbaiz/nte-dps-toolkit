param([switch]$CheckOnly)

$ErrorActionPreference = "Stop"
$artifactRoot = $PSScriptRoot
$workspaceRoot = (Resolve-Path -LiteralPath (Join-Path $artifactRoot "../..")).Path
$modifiedManifest = Join-Path $artifactRoot "modified.sha256"
$baselineManifest = Join-Path $artifactRoot "baseline.sha256"
$originalRoot = Join-Path $artifactRoot "original"

function Read-Manifest {
    param([string]$Path)
    foreach ($line in Get-Content -LiteralPath $Path) {
        if ($line -match '^([0-9a-f]{64})  (.+)$') {
            [pscustomobject]@{ Hash = $Matches[1]; RelativePath = $Matches[2]; Missing = $false }
        } elseif ($line -match '^MISSING  (.+)$') {
            [pscustomobject]@{ Hash = $null; RelativePath = $Matches[1]; Missing = $true }
        } elseif ($line.Trim().Length -gt 0) {
            throw "Invalid manifest line: $line"
        }
    }
}

foreach ($entry in Read-Manifest $modifiedManifest) {
    $path = Join-Path $workspaceRoot $entry.RelativePath
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Modified file is missing: $($entry.RelativePath)"
    }
    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $entry.Hash) {
        throw "Modified hash mismatch: $($entry.RelativePath)"
    }
}
Write-Output "modified-precondition=PASS"

if ($CheckOnly) {
    Write-Output "rollback-check-only=PASS"
    exit 0
}

foreach ($entry in Read-Manifest $baselineManifest) {
    $target = Join-Path $workspaceRoot $entry.RelativePath
    if ($entry.Missing) {
        if (Test-Path -LiteralPath $target -PathType Leaf) {
            Remove-Item -LiteralPath $target -Force
        }
        continue
    }
    $source = Join-Path $originalRoot $entry.RelativePath
    $parent = Split-Path -Parent $target
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        New-Item -ItemType Directory -Path $parent | Out-Null
    }
    Copy-Item -LiteralPath $source -Destination $target -Force
}

foreach ($entry in Read-Manifest $baselineManifest) {
    $path = Join-Path $workspaceRoot $entry.RelativePath
    if ($entry.Missing) {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            throw "Rollback expected an absent file: $($entry.RelativePath)"
        }
        continue
    }
    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $entry.Hash) {
        throw "Baseline hash mismatch: $($entry.RelativePath)"
    }
}
Write-Output "rollback=PASS"
