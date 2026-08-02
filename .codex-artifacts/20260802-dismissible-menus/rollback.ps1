param(
    [Parameter(Mandatory = $true)]
    [string]$TargetRoot
)

$ErrorActionPreference = "Stop"
$sourceRoot = Join-Path $PSScriptRoot "original"
$resolvedTarget = [System.IO.Path]::GetFullPath($TargetRoot)
$restore = @(
    "frontend/src/features/main-dps/main-dps-page.tsx",
    "frontend/src/features/main-dps/main-dps-detail-page.tsx",
    "frontend/src/features/technical-hud/technical-hud-page.tsx"
)
$remove = @(
    "frontend/src/hooks/use-dismissible-layer.ts",
    "frontend/src/hooks/use-dismissible-layer.test.ts"
)

foreach ($relativePath in $restore) {
    $source = Join-Path $sourceRoot $relativePath
    $destination = Join-Path $resolvedTarget $relativePath
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Missing preserved original: $relativePath"
    }
    New-Item -ItemType Directory -Force -Path (Split-Path $destination) | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination -Force
}

foreach ($relativePath in $remove) {
    $destination = Join-Path $resolvedTarget $relativePath
    if (Test-Path -LiteralPath $destination -PathType Leaf) {
        Remove-Item -LiteralPath $destination -Force
    }
}

Write-Output "ROLLBACK_VERIFIED restored=$($restore.Count) removed=$($remove.Count)"
