param(
    [switch]$CheckOnly
)

$ErrorActionPreference = "Stop"
$artifactRoot = $PSScriptRoot
$workspace = [System.IO.Path]::GetFullPath((Join-Path $artifactRoot "..\.."))
$manifestPath = Join-Path $artifactRoot "original-hashes.json"
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json

if (-not (Test-Path -LiteralPath (Join-Path $workspace "Cargo.toml"))) {
    throw "Rollback target is not the expected workspace: $workspace"
}

foreach ($entry in $manifest) {
    $relative = $entry.Path.Replace("/", [System.IO.Path]::DirectorySeparatorChar)
    $source = Join-Path (Join-Path $artifactRoot "original") $relative
    $actual = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    if ($actual -ne $entry.Sha256) {
        throw "Original snapshot hash mismatch: $($entry.Path)"
    }
}

if ($CheckOnly) {
    Write-Output "Rollback check passed: $($manifest.Count) original files verified for $workspace"
    exit 0
}

foreach ($entry in $manifest) {
    $relative = $entry.Path.Replace("/", [System.IO.Path]::DirectorySeparatorChar)
    $source = Join-Path (Join-Path $artifactRoot "original") $relative
    $target = Join-Path $workspace $relative
    New-Item -ItemType Directory -Force -Path (Split-Path $target) | Out-Null
    Copy-Item -Force -LiteralPath $source -Destination $target
    $actual = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash
    if ($actual -ne $entry.Sha256) {
        throw "Rollback verification failed: $($entry.Path)"
    }
}

Write-Output "Rollback complete: restored and verified $($manifest.Count) files."
