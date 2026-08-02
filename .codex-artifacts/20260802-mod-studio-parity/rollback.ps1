param(
    [string]$WorkspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$originalRoot = Join-Path $PSScriptRoot 'original'
$hashes = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'modified-hashes.json') -Raw | ConvertFrom-Json

foreach ($entry in $hashes.PSObject.Properties) {
    $relativePath = $entry.Name
    $target = Join-Path $WorkspaceRoot ($relativePath -replace '/', [IO.Path]::DirectorySeparatorChar)
    if (-not $Force) {
        if (-not (Test-Path -LiteralPath $target -PathType Leaf)) {
            throw "Rollback guard: missing modified file: $relativePath"
        }
        $actual = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne [string]$entry.Value) {
            throw "Rollback guard: file changed after this task: $relativePath"
        }
    }
}

foreach ($entry in $hashes.PSObject.Properties) {
    $relativePath = $entry.Name
    $relativeNative = $relativePath -replace '/', [IO.Path]::DirectorySeparatorChar
    $source = Join-Path $originalRoot $relativeNative
    $target = Join-Path $WorkspaceRoot $relativeNative
    $parent = Split-Path -Parent $target
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    Copy-Item -LiteralPath $source -Destination $target -Force
}

Write-Output "ROLLBACK_RESTORED=$(@($hashes.PSObject.Properties).Count)"
Write-Output "ROLLBACK_ROOT=$([IO.Path]::GetFullPath($WorkspaceRoot))"

