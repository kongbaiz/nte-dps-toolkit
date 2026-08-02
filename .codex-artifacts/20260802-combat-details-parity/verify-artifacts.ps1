$ErrorActionPreference = "Stop"
$artifactRoot = $PSScriptRoot
$workspace = [System.IO.Path]::GetFullPath((Join-Path $artifactRoot "..\.."))
$capabilityFiles = @(
    "src-tauri/capabilities/abyss-values.json",
    "src-tauri/capabilities/combat-details.json",
    "src-tauri/capabilities/console.json",
    "src-tauri/capabilities/main-dps.json"
)

function SnapshotPath([string]$snapshot, [string]$relative) {
    return Join-Path (Join-Path $artifactRoot $snapshot) $relative.Replace("/", "\")
}

function WindowControlCapabilityCount([string]$snapshot) {
    $count = 0
    foreach ($relative in $capabilityFiles) {
        $json = Get-Content -LiteralPath (SnapshotPath $snapshot $relative) -Raw | ConvertFrom-Json
        $required = @(
            "core:window:allow-minimize",
            "core:window:allow-toggle-maximize",
            "core:window:allow-close"
        )
        if (($required | Where-Object { $_ -notin $json.permissions }).Count -eq 0) {
            $count += 1
        }
    }
    return $count
}

function ContractVersion([string]$snapshot) {
    $path = SnapshotPath $snapshot "src-tauri/src/contract/main_dps_detail.rs"
    $match = Select-String -LiteralPath $path -Pattern "MAIN_DPS_DETAIL_CONTRACT_VERSION: u32 = ([0-9]+)" | Select-Object -First 1
    if (-not $match) { throw "Detail contract version missing in $snapshot" }
    return $match.Matches[0].Groups[1].Value
}

function PageHas([string]$snapshot, [string]$pattern) {
    $path = SnapshotPath $snapshot "frontend/src/features/main-dps/main-dps-detail-page.tsx"
    return [bool](Select-String -LiteralPath $path -SimpleMatch $pattern -Quiet)
}

foreach ($manifestName in @("original-hashes.json", "modified-hashes.json")) {
    $snapshot = if ($manifestName -eq "original-hashes.json") { "original" } else { "modified-snapshot" }
    $manifest = Get-Content -LiteralPath (Join-Path $artifactRoot $manifestName) -Raw | ConvertFrom-Json
    foreach ($entry in $manifest) {
        $path = SnapshotPath $snapshot $entry.Path
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $entry.Sha256) {
            throw "$snapshot hash mismatch: $($entry.Path)"
        }
    }
}

$modifiedManifest = Get-Content -LiteralPath (Join-Path $artifactRoot "modified-hashes.json") -Raw | ConvertFrom-Json
foreach ($entry in $modifiedManifest) {
    $path = Join-Path $workspace $entry.Path.Replace("/", "\")
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $entry.Sha256) {
        throw "Workspace differs from modified snapshot: $($entry.Path)"
    }
}

Write-Output "BASELINE contractVersion=$(ContractVersion 'original') windowControlCapabilities=$(WindowControlCapabilityCount 'original')/4 summary=$((PageHas 'original' 'DetailSummary snapshot=').ToString().ToLower()) skillBreakdown=$((PageHas 'original' '<SkillBreakdown').ToString().ToLower()) targetHp=$((PageHas 'original' 'targetHpPercent').ToString().ToLower())"
Write-Output "MODIFIED contractVersion=$(ContractVersion 'modified-snapshot') windowControlCapabilities=$(WindowControlCapabilityCount 'modified-snapshot')/4 summary=$((PageHas 'modified-snapshot' 'DetailSummary snapshot=').ToString().ToLower()) skillBreakdown=$((PageHas 'modified-snapshot' '<SkillBreakdown').ToString().ToLower()) targetHp=$((PageHas 'modified-snapshot' 'targetHpPercent').ToString().ToLower())"
Write-Output "ARTIFACTS original=verified modified=verified workspace=matches-modified files=$($modifiedManifest.Count)"
