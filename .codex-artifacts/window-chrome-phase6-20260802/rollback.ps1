[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Root
)

$ErrorActionPreference = "Stop"
$artifactRoot = $PSScriptRoot
$manifestPath = Join-Path $artifactRoot "baseline-manifest.json"
$originalsRoot = Join-Path $artifactRoot "originals"
$resolvedRoot = [System.IO.Path]::GetFullPath($Root)
$rootPrefix = $resolvedRoot.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json

foreach ($item in $manifest) {
    $relativePath = [string]$item.path
    $destination = [System.IO.Path]::GetFullPath((Join-Path $resolvedRoot $relativePath))
    if (-not $destination.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Rollback destination escaped Root: $relativePath"
    }

    if ([bool]$item.existed) {
        $source = Join-Path $originalsRoot $relativePath
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Missing preserved original: $source"
        }
        $destinationDirectory = Split-Path -Parent $destination
        New-Item -ItemType Directory -Force -Path $destinationDirectory | Out-Null
        Copy-Item -LiteralPath $source -Destination $destination -Force
    }
    elseif (Test-Path -LiteralPath $destination) {
        Remove-Item -LiteralPath $destination -Force
    }
}

foreach ($item in $manifest) {
    $destination = [System.IO.Path]::GetFullPath((Join-Path $resolvedRoot ([string]$item.path)))
    if ([bool]$item.existed) {
        $actual = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
        if ($actual -ne [string]$item.sha256) {
            throw "Rollback hash mismatch: $($item.path)"
        }
        Write-Output "restored $($item.path) $actual"
    }
    elseif (Test-Path -LiteralPath $destination) {
        throw "Rollback expected absence: $($item.path)"
    }
    else {
        Write-Output "removed $($item.path)"
    }
}

Write-Output "rollback verified root=$resolvedRoot"
