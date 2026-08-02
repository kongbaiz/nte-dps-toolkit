param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("Baseline", "Modified")]
    [string]$Snapshot
)

$ErrorActionPreference = "Stop"
$artifactRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$snapshotRoot = if ($Snapshot -eq "Baseline") {
    Join-Path $artifactRoot "baseline"
}
else {
    Join-Path $artifactRoot "modified-snapshot"
}

function Contains([string]$relativePath, [string]$pattern) {
    $path = Join-Path $snapshotRoot $relativePath
    (Get-Content -LiteralPath $path -Raw).Contains($pattern)
}

if ($Snapshot -eq "Baseline") {
    $checks = [ordered]@{
        detail_actions_disabled = Contains "src-tauri/src/contract/main_dps.rs" "character_details_available: false"
        team_detail_client_absent = -not (Contains "frontend/src/lib/tauri/main-dps-client.ts" "openTeamDetails")
        attribution_not_buttons = -not (Contains "frontend/src/features/main-dps/main-dps-page.tsx" 'team-details-${filter}')
        detail_route_absent = -not (Test-Path -LiteralPath (Join-Path $snapshotRoot "frontend/src/features/main-dps/main-dps-detail-page.tsx"))
        independent_monaco_theme = Contains "frontend/src/features/mod-studio/mod-studio-page.tsx" "setEditorTheme"
        blue_native_dark_tint = Contains "frontend/src/lib/tauri/console-window-background.ts" "[1, 6, 15, 255]"
    }
}
else {
    $config = Get-Content -LiteralPath (Join-Path $snapshotRoot "src-tauri/tauri.conf.json") -Raw | ConvertFrom-Json
    $console = $config.app.windows | Where-Object label -eq "console"
    $abyss = $config.app.windows | Where-Object label -eq "abyss-values"
    $detail = $config.app.windows | Where-Object label -eq "combat-details"
    $checks = [ordered]@{
        detail_actions_enabled_from_hits = Contains "src-tauri/src/contract/main_dps.rs" "character_details_available: has_hits"
        team_detail_command_wired = Contains "frontend/src/lib/tauri/main-dps-client.ts" "openTeamDetails"
        attribution_buttons_wired = Contains "frontend/src/features/main-dps/main-dps-page.tsx" 'team-details-${filter}'
        detail_route_present = Test-Path -LiteralPath (Join-Path $snapshotRoot "frontend/src/features/main-dps/main-dps-detail-page.tsx")
        auto_half_transition_test = Contains "src-tauri/src/state.rs" "live_abyss_selection_follows_only_real_half_transitions"
        compact_height_container_queries = Contains "frontend/src/index.css" "@container main-dps-character-row (max-height: 2.25rem)"
        monaco_follows_interface_theme = Contains "frontend/src/features/mod-studio/mod-studio-page.tsx" 'useSettingsPresentation().darkMode ? "dark" : "light"'
        neutral_native_dark_tint = Contains "frontend/src/lib/tauri/console-window-background.ts" "[9, 9, 11, 255]"
        console_custom_titlebar = $console.decorations -eq $false
        abyss_custom_titlebar = $abyss.decorations -eq $false
        detail_custom_titlebar = $detail.decorations -eq $false
    }
}

$checks.GetEnumerator() | ForEach-Object {
    Write-Output "$($_.Key)=$($_.Value.ToString().ToLowerInvariant())"
}
if ($checks.Values -contains $false) {
    exit 1
}
