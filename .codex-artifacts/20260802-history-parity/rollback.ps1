param(
    [Parameter(Mandatory = $true)]
    [string]$Root
)

$ErrorActionPreference = 'Stop'
$artifact = $PSScriptRoot
$resolvedRoot = (Resolve-Path -LiteralPath $Root).Path
$modified = Get-Content -Raw -LiteralPath (Join-Path $artifact 'modified-hashes.json') | ConvertFrom-Json -AsHashtable
$original = Get-Content -Raw -LiteralPath (Join-Path $artifact 'original-hashes.json') | ConvertFrom-Json -AsHashtable

foreach ($relative in $modified.Keys) {
    $target = Join-Path $resolvedRoot $relative
    if (-not (Test-Path -LiteralPath $target -PathType Leaf)) {
        throw "Rollback guard failed: missing $relative"
    }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $target).Hash.ToLowerInvariant()
    if ($actual -ne $modified[$relative]) {
        throw "Rollback guard failed: modified hash mismatch for $relative"
    }
}

foreach ($relative in $original.Keys) {
    $source = Join-Path (Join-Path $artifact 'original') $relative
    $target = Join-Path $resolvedRoot $relative
    $parent = Split-Path -Parent $target
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    Copy-Item -Force -LiteralPath $source -Destination $target
}

foreach ($relative in $original.Keys) {
    $target = Join-Path $resolvedRoot $relative
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $target).Hash.ToLowerInvariant()
    if ($actual -ne $original[$relative]) {
        throw "Rollback verification failed for $relative"
    }
}

Write-Output ("rollback_verified_files={0}" -f $original.Count)
