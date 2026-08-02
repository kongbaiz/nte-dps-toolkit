param(
    [string]$WorkspaceRoot = "D:\NTE_DPS_TOOL"
)

$ErrorActionPreference = "Stop"
$artifactRoot = $PSScriptRoot
$manifest = Get-Content (Join-Path $artifactRoot "baseline-manifest.json") -Raw | ConvertFrom-Json
foreach ($entry in $manifest) {
    $source = Join-Path (Join-Path $artifactRoot "original") $entry.path
    $target = Join-Path $WorkspaceRoot $entry.path
    New-Item -ItemType Directory -Force -Path (Split-Path $target) | Out-Null
    Copy-Item -LiteralPath $source -Destination $target -Force
}

$newFiles = @(
    "frontend/src/hooks/use-hud-module-pointer-reorder.ts",
    "frontend/src/hooks/use-hud-module-pointer-reorder.test.ts",
    "docs/TAURI_MIGRATION_PHASE34.md"
)
foreach ($relativePath in $newFiles) {
    $target = Join-Path $WorkspaceRoot $relativePath
    if (Test-Path -LiteralPath $target) {
        Remove-Item -LiteralPath $target -Force
    }
}

foreach ($entry in $manifest) {
    $target = Join-Path $WorkspaceRoot $entry.path
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $target).Hash.ToLowerInvariant()
    if ($actual -ne $entry.sha256) {
        throw "Rollback hash mismatch: $($entry.path)"
    }
}
Write-Output "ROLLBACK_RESTORED=$($manifest.Count)"
Write-Output "ROLLBACK_REMOVED=$($newFiles.Count)"
exit 0
