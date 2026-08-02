param(
    [Parameter(Mandatory = $true)]
    [string]$Root,
    [Parameter(Mandatory = $true)]
    [ValidateSet("baseline", "modified")]
    [string]$Mode
)

$ErrorActionPreference = "Stop"
$resolvedRoot = (Resolve-Path -LiteralPath $Root).Path
$failed = $false

function Test-Probe {
    param(
        [string]$Name,
        [string]$RelativePath,
        [string]$Pattern,
        [bool]$Expected
    )

    $path = Join-Path $resolvedRoot $RelativePath
    $actual = Test-Path -LiteralPath $path -PathType Leaf
    if ($actual) {
        $actual = [regex]::IsMatch(
            [System.IO.File]::ReadAllText($path),
            $Pattern,
            [System.Text.RegularExpressions.RegexOptions]::Multiline
        )
    }
    Write-Output "$Name=$($actual.ToString().ToLowerInvariant()) expected=$($Expected.ToString().ToLowerInvariant())"
    if ($actual -ne $Expected) {
        $script:failed = $true
    }
}

if ($Mode -eq "baseline") {
    Test-Probe "abyss-separate-current-button" "frontend/src/features/abyss-values/abyss-values-page.tsx" 't\("Import Current"\)' $true
    Test-Probe "abyss-duplicate-team-roster" "frontend/src/features/abyss-values/abyss-values-page.tsx" 'function TeamRoster' $true
    Test-Probe "replay-atomic-content-gate" "frontend/src/features/main-dps/main-dps-page.tsx" 'function ReplayImportLoading' $false
    Test-Probe "hud-direct-module-drag" "frontend/src/features/technical-hud/technical-hud-page.tsx" 'function HudEditableModule' $false
    Test-Probe "hud-character-avatar" "frontend/src/features/technical-hud/technical-hud-page.tsx" 'characterAvatarUrl\(character\.characterId\)' $false
    Test-Probe "rank-independent-color-fallback" "frontend/src/features/main-dps/main-dps-model.ts" 'Math\.abs\(characterId\) % characterFallbackColors\.length' $true
} else {
    Test-Probe "abyss-inline-team-avatars" "frontend/src/features/abyss-values/abyss-values-page.tsx" 'function AbyssTeamAvatars' $true
    Test-Probe "abyss-current-team-availability" "frontend/src/lib/tauri/abyss-values-contract.ts" 'currentTeamAvailable' $true
    Test-Probe "replay-atomic-content-gate" "frontend/src/features/main-dps/main-dps-page.tsx" 'function ReplayImportLoading' $true
    Test-Probe "hud-direct-module-drag" "frontend/src/features/technical-hud/technical-hud-page.tsx" 'function HudEditableModule' $true
    Test-Probe "hud-character-avatar" "frontend/src/features/technical-hud/technical-hud-page.tsx" 'characterAvatarUrl\(character\.characterId\)' $true
    Test-Probe "shared-avatar-color-projection" "src/storage/resource.rs" 'fill_missing_character_colors_from_avatars' $true
    Test-Probe "known-character-color-regression" "src/storage/resource.rs" 'Some\("#A72648"\)' $true
    Test-Probe "hud-rank-independent-fallback" "frontend/src/features/technical-hud/technical-hud-page.tsx" 'characterAccent\(character\.characterId, character\.color\)' $true
}

if ($failed) {
    exit 1
}
exit 0
