param(
    [Parameter(Mandatory = $true)]
    [string]$TargetRoot
)

$ErrorActionPreference = "Stop"
$artifactRoot = $PSScriptRoot
$sourceRoot = Join-Path $artifactRoot "original"
$resolvedTarget = [System.IO.Path]::GetFullPath($TargetRoot)
$files = @(
    "frontend/src/features/main-dps/main-dps-page.tsx",
    "frontend/src/features/main-dps/main-dps-model.ts",
    "frontend/src/features/main-dps/main-dps-model.test.ts"
)

foreach ($relativePath in $files) {
    $source = Join-Path $sourceRoot $relativePath
    $destination = Join-Path $resolvedTarget $relativePath
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Missing preserved original: $relativePath"
    }
    New-Item -ItemType Directory -Force -Path (Split-Path $destination) | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination -Force
}

Write-Output "ROLLBACK_VERIFIED files=$($files.Count)"
