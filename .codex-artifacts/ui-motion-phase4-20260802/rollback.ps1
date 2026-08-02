$ErrorActionPreference = 'Stop'

$workspace = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$original = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'original'))
if (-not $original.StartsWith($PSScriptRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Original snapshot resolved outside the phase artifact directory: $original"
}

$restore = @(
    'frontend/src/components/nte/window-motion-boundary.tsx',
    'frontend/src/features/main-dps/main-dps-page.tsx',
    'frontend/src/features/main-dps/use-main-dps.ts',
    'frontend/src/features/technical-hud/technical-hud-page.tsx',
    'frontend/src/features/technical-hud/use-technical-state.ts',
    'src-tauri/src/commands/main_dps.rs',
    'src-tauri/src/commands/technical.rs',
    'src-tauri/src/windows/island.rs'
)
$remove = @(
    'frontend/src/components/nte/window-motion-boundary.test.ts',
    'frontend/src/components/nte/window-motion-target.ts'
)

foreach ($relative in $restore) {
    $source = [System.IO.Path]::GetFullPath((Join-Path $original $relative))
    $target = [System.IO.Path]::GetFullPath((Join-Path $workspace $relative))
    if (-not $source.StartsWith($original, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Snapshot source resolved outside the original directory: $source"
    }
    if (-not $target.StartsWith($workspace, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Rollback target resolved outside the workspace: $target"
    }
    Copy-Item -LiteralPath $source -Destination $target -Force
}

foreach ($relative in $remove) {
    $target = [System.IO.Path]::GetFullPath((Join-Path $workspace $relative))
    if (-not $target.StartsWith($workspace, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Rollback removal resolved outside the workspace: $target"
    }
    if (Test-Path -LiteralPath $target) {
        Remove-Item -LiteralPath $target -Force
    }
}

foreach ($line in Get-Content -LiteralPath (Join-Path $PSScriptRoot 'original-sha256.txt')) {
    if ($line -notmatch '^([0-9a-f]{64})  (.+)$') {
        throw "Invalid original hash manifest line: $line"
    }
    $expected = $Matches[1]
    $relative = $Matches[2]
    $target = Join-Path $workspace $relative
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $target).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "Rollback hash mismatch: $relative"
    }
}

Write-Output "Rollback complete: original files restored and phase-created files removed."
