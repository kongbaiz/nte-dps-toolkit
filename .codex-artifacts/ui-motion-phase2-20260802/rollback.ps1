param(
    [string]$WorkspaceRoot = [System.IO.Path]::GetFullPath(
        (Join-Path $PSScriptRoot '..\..')
    )
)

$ErrorActionPreference = 'Stop'
$artifactRoot = [System.IO.Path]::GetFullPath($PSScriptRoot)
$originalRoot = Join-Path $artifactRoot 'original'
$workspacePrefix = $WorkspaceRoot.TrimEnd('\') + '\'

$existingFiles = @(
    'frontend\src\index.css',
    'frontend\src\App.tsx',
    'frontend\src\components\ui\skeleton.tsx',
    'frontend\src\features\console\console-page.tsx',
    'frontend\src\features\main-dps\main-dps-page.tsx',
    'frontend\src\features\main-dps\use-main-dps.ts',
    'frontend\src\features\technical-hud\technical-hud-page.tsx',
    'frontend\src\features\technical-hud\use-technical-state.ts',
    'frontend\src\features\island\island-page.tsx',
    'res\languages\zh-CN.json',
    'res\languages\ja.json',
    'src-tauri\src\commands\main_dps.rs',
    'src-tauri\src\windows\mod.rs'
)

$newFiles = @(
    'frontend\src\components\nte\action-notice.tsx',
    'frontend\src\components\nte\animated-number.tsx',
    'frontend\src\components\nte\motion-route-loading.tsx',
    'frontend\src\components\nte\window-motion-boundary.tsx',
    'frontend\src\components\nte\window-motion-context.ts',
    'frontend\src\lib\motion.ts',
    'frontend\src\lib\motion.test.ts'
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
    $targetDirectory = Split-Path -Parent $target
    if (-not (Test-Path -LiteralPath $targetDirectory -PathType Container)) {
        New-Item -ItemType Directory -Path $targetDirectory | Out-Null
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

Write-Host "Rollback restored $($existingFiles.Count) files and removed $($newFiles.Count) motion-only files."
