param(
    [string]$WorkspaceRoot = [System.IO.Path]::GetFullPath(
        (Join-Path $PSScriptRoot '..\..')
    )
)

$ErrorActionPreference = 'Stop'
$originalRoot = Join-Path $PSScriptRoot 'original'
$workspacePrefix = $WorkspaceRoot.TrimEnd('\') + '\'
$existingFiles = @(
    'frontend\src\index.css',
    'frontend\src\features\console\console-sidebar.tsx',
    'frontend\src\features\technical-hud\technical-hud-page.tsx',
    'frontend\src\features\island\island-page.tsx',
    'frontend\src\features\main-dps\main-dps-page.tsx',
    'frontend\src\lib\motion.ts',
    'src-tauri\src\windows\island.rs'
)
$newFiles = @(
    'frontend\src\hooks\use-layout-flip.ts',
    'frontend\src\hooks\use-layout-flip.test.ts'
)

function Resolve-WorkspaceTarget([string]$RelativePath) {
    $target = [System.IO.Path]::GetFullPath((Join-Path $WorkspaceRoot $RelativePath))
    if (-not $target.StartsWith($workspacePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Rollback target left the workspace: $target"
    }
    return $target
}

foreach ($relativePath in $existingFiles) {
    $source = Join-Path $originalRoot $relativePath
    $target = Resolve-WorkspaceTarget $relativePath
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Missing preserved original: $source"
    }
    Copy-Item -LiteralPath $source -Destination $target -Force
}

foreach ($relativePath in $newFiles) {
    $target = Resolve-WorkspaceTarget $relativePath
    if (Test-Path -LiteralPath $target -PathType Leaf) {
        Remove-Item -LiteralPath $target -Force
    }
}

foreach ($relativePath in $existingFiles) {
    $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $originalRoot $relativePath)).Hash
    $targetHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Resolve-WorkspaceTarget $relativePath)).Hash
    if ($sourceHash -ne $targetHash) {
        throw "Rollback hash mismatch: $relativePath"
    }
}

Write-Host "Rollback restored $($existingFiles.Count) files and removed $($newFiles.Count) phase-three files."
