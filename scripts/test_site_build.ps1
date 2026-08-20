param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$buildScript = Join-Path $PSScriptRoot "build_site.ps1"

if (-not (Test-Path -LiteralPath $buildScript -PathType Leaf)) {
    throw "Site build script is missing: $buildScript"
}

$trackedSiteAssets = @(& git -C $repoRoot ls-files -- "site/assets/img/**")
if ($LASTEXITCODE -ne 0) {
    throw "git ls-files failed with exit code $LASTEXITCODE"
}
$presentTrackedSiteAssets = @(
    $trackedSiteAssets | Where-Object {
        Test-Path -LiteralPath (Join-Path $repoRoot $_) -PathType Leaf
    }
)
if ($presentTrackedSiteAssets.Count -ne 0) {
    throw "Generated site image copies remain in the worktree: $($presentTrackedSiteAssets.Count)"
}

$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$outputDirectory = Join-Path $tempRoot ("nte-dps-site-{0}" -f [Guid]::NewGuid().ToString("N"))

try {
    & $buildScript -OutputDirectory $outputDirectory
    if ($LASTEXITCODE -ne 0) {
        throw "Site build failed with exit code $LASTEXITCODE"
    }

    $expectedAssets = [Collections.Generic.List[object]]::new()
    $imagesRoot = Join-Path $repoRoot "images"
    foreach ($source in Get-ChildItem -LiteralPath $imagesRoot -File -Recurse) {
        $relativePath = [IO.Path]::GetRelativePath($imagesRoot, $source.FullName)
        $expectedAssets.Add([pscustomobject]@{
                Source      = $source.FullName
                Destination = Join-Path $outputDirectory (Join-Path "assets/img" $relativePath)
            })
    }
    $expectedAssets.Add([pscustomobject]@{
            Source      = Join-Path $repoRoot "res/icons/app-icon.png"
            Destination = Join-Path $outputDirectory "assets/img/app-icon.png"
        })

    foreach ($asset in $expectedAssets) {
        if (-not (Test-Path -LiteralPath $asset.Destination -PathType Leaf)) {
            throw "Built site asset is missing: $($asset.Destination)"
        }
        $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $asset.Source).Hash
        $destinationHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $asset.Destination).Hash
        if ($sourceHash -ne $destinationHash) {
            throw "Built site asset differs from its canonical source: $($asset.Destination)"
        }
    }

    foreach ($staticFile in @("index.html", "robots.txt", "sitemap.xml")) {
        $source = Join-Path $repoRoot (Join-Path "site" $staticFile)
        $destination = Join-Path $outputDirectory $staticFile
        if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) {
            throw "Built site file is missing: $destination"
        }
        if ((Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash -ne
            (Get-FileHash -Algorithm SHA256 -LiteralPath $destination).Hash) {
            throw "Built site file differs from its source: $destination"
        }
    }

    $builtFiles = @(Get-ChildItem -LiteralPath $outputDirectory -File -Recurse)
    $expectedFileCount = $expectedAssets.Count + 3
    if ($builtFiles.Count -ne $expectedFileCount) {
        throw "Built site file set is not deterministic: expected $expectedFileCount, got $($builtFiles.Count)"
    }

    $overwriteRejected = $false
    try {
        & $buildScript -OutputDirectory $outputDirectory
    } catch {
        if ($_.Exception.Message -notlike "Site output already exists; use a new empty path:*") {
            throw
        }
        $overwriteRejected = $true
    }
    if (-not $overwriteRejected) {
        throw "Site build unexpectedly overwrote an existing output directory"
    }

    Write-Output (
        "SITE-BUILD-VERIFY: OK files={0} assets={1} trackedDuplicateBytes=0 overwriteRejected=true" -f
        $builtFiles.Count,
        $expectedAssets.Count
    )
} finally {
    $resolvedOutput = [IO.Path]::GetFullPath($outputDirectory)
    if ($resolvedOutput.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path -Leaf $resolvedOutput).StartsWith("nte-dps-site-", [StringComparison]::Ordinal)) {
        Remove-Item -LiteralPath $resolvedOutput -Recurse -Force -ErrorAction SilentlyContinue
    }
}
