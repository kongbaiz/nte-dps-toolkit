param(
    [switch]$Apply
)

$ErrorActionPreference = "Stop"
$ArtifactRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent (Split-Path -Parent $ArtifactRoot)
$BaselineRoot = Join-Path $ArtifactRoot "baseline"
$ModifiedRoot = Join-Path $ArtifactRoot "modified-snapshot"

$baselineManifest = Get-Content -Raw -LiteralPath (Join-Path $ArtifactRoot "baseline-manifest.json") | ConvertFrom-Json
$modifiedManifest = Get-Content -Raw -LiteralPath (Join-Path $ArtifactRoot "modified-manifest.json") | ConvertFrom-Json

foreach ($entry in $baselineManifest) {
    $baselinePath = Join-Path $BaselineRoot $entry.path
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $baselinePath).Hash.ToLowerInvariant()
    if ($actual -ne $entry.sha256) {
        throw "Baseline artifact hash mismatch: $($entry.path)"
    }
}

foreach ($entry in $modifiedManifest) {
    $currentPath = Join-Path $RepoRoot $entry.path
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $currentPath).Hash.ToLowerInvariant()
    if ($actual -ne $entry.sha256) {
        throw "Current file changed after verification; rollback stopped: $($entry.path)"
    }
}

foreach ($entry in $baselineManifest) {
    $source = Join-Path $BaselineRoot $entry.path
    $destination = Join-Path $RepoRoot $entry.path
    if ($Apply) {
        Copy-Item -LiteralPath $source -Destination $destination -Force
        $restored = (Get-FileHash -Algorithm SHA256 -LiteralPath $destination).Hash.ToLowerInvariant()
        if ($restored -ne $entry.sha256) {
            throw "Restored file hash mismatch: $($entry.path)"
        }
        Write-Output "RESTORED $($entry.path)"
    } else {
        Write-Output "WOULD_RESTORE $($entry.path)"
    }
}

if ($Apply) {
    Write-Output "ROLLBACK_APPLIED_OK"
} else {
    Write-Output "ROLLBACK_DRY_RUN_OK"
}
