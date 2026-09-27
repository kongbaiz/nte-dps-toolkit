param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$outputDirectory = Join-Path $tempRoot ("nte-dps-site-{0}" -f [Guid]::NewGuid().ToString("N"))

try {
    & (Join-Path $PSScriptRoot "build_site.ps1") -OutputDirectory $outputDirectory

    foreach ($file in @("index.html", "robots.txt", "sitemap.xml", "assets/img/app-icon.png")) {
        $path = Join-Path $outputDirectory $file
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Built site file is missing: $path"
        }
    }

    $sourceAssetCount = @(Get-ChildItem -LiteralPath (Join-Path $repoRoot "images") -File -Recurse).Count + 1
    $builtAssetCount = @(Get-ChildItem -LiteralPath (Join-Path $outputDirectory "assets/img") -File -Recurse).Count
    if ($builtAssetCount -ne $sourceAssetCount) {
        throw "Built site assets differ: expected $sourceAssetCount, got $builtAssetCount"
    }

    Write-Output "SITE-BUILD-VERIFY: OK assets=$builtAssetCount"
}
finally {
    if ((Test-Path -LiteralPath $outputDirectory) -and
        $outputDirectory.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path -Leaf $outputDirectory).StartsWith("nte-dps-site-", [StringComparison]::Ordinal)) {
        Remove-Item -LiteralPath $outputDirectory -Recurse -Force
    }
}
