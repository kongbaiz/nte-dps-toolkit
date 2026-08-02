param(
    [switch]$Apply
)

$ErrorActionPreference = "Stop"
$artifactRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent (Split-Path -Parent $artifactRoot)
$baselineRoot = Join-Path $artifactRoot "baseline"
$manifestPath = Join-Path $artifactRoot "baseline-manifest-expanded.json"

if (-not (Test-Path -LiteralPath (Join-Path $repoRoot "Cargo.toml"))) {
    throw "Rollback root validation failed: Cargo.toml was not found."
}
if (-not (Test-Path -LiteralPath $manifestPath)) {
    throw "Rollback manifest is missing."
}

$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
foreach ($entry in $manifest) {
    $source = Join-Path $baselineRoot $entry.path
    $actualHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $entry.sha256) {
        throw "Baseline hash mismatch: $($entry.path)"
    }

    $destination = Join-Path $repoRoot $entry.path
    if ($Apply) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
        Copy-Item -LiteralPath $source -Destination $destination -Force
        Write-Output "RESTORED $($entry.path)"
    }
    else {
        Write-Output "WOULD_RESTORE $($entry.path)"
    }
}

$createdFiles = @(
    "src/core/combat_details.rs",
    "src-tauri/src/contract/main_dps_detail.rs",
    "src-tauri/src/windows/combat_details.rs",
    "src-tauri/capabilities/combat-details.json",
    "frontend/src/lib/tauri/desktop-window-client.ts",
    "frontend/src/lib/tauri/desktop-window-client.test.ts",
    "frontend/src/components/nte/desktop-titlebar.tsx",
    "frontend/src/lib/tauri/main-dps-detail-contract.ts",
    "frontend/src/lib/tauri/main-dps-detail-contract.test.ts",
    "frontend/src/lib/tauri/main-dps-detail-client.ts",
    "frontend/src/features/main-dps/main-dps-detail-page.tsx"
)

foreach ($relativePath in $createdFiles) {
    $target = Join-Path $repoRoot $relativePath
    if (-not (Test-Path -LiteralPath $target)) {
        continue
    }
    if ($Apply) {
        Remove-Item -LiteralPath $target -Force
        Write-Output "REMOVED $relativePath"
    }
    else {
        Write-Output "WOULD_REMOVE $relativePath"
    }
}

Write-Output $(if ($Apply) { "ROLLBACK_APPLIED" } else { "ROLLBACK_DRY_RUN_OK" })
