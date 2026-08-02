param(
    [string]$TargetRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
)
$ErrorActionPreference = 'Stop'
$originalRoot = Join-Path $PSScriptRoot 'original'
Get-ChildItem -LiteralPath $originalRoot -Recurse -File | ForEach-Object {
    $relative = [System.IO.Path]::GetRelativePath($originalRoot, $_.FullName)
    $destination = Join-Path $TargetRoot $relative
    New-Item -ItemType Directory -Force (Split-Path $destination) | Out-Null
    Copy-Item -LiteralPath $_.FullName -Destination $destination -Force
}
$absent = Get-Content (Join-Path $PSScriptRoot 'original-absent.txt') | Where-Object { $_ }
$absent | ForEach-Object {
    $path = Join-Path $TargetRoot $_
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Force
    }
}
$manifest = Get-Content (Join-Path $PSScriptRoot 'original-manifest.json') -Raw | ConvertFrom-Json
foreach ($entry in $manifest) {
    $path = Join-Path $TargetRoot $entry.path
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant()
    if ($actual -ne $entry.sha256) {
        throw "Rollback hash mismatch: $($entry.path)"
    }
}
Write-Output "ROLLBACK_VERIFIED files=$($manifest.Count) removed=$($absent.Count)"
