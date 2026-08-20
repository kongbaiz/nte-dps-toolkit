param(
    [string]$OutputDirectory = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot "target/site"
}
$resolvedOutput = [IO.Path]::GetFullPath($OutputDirectory)

function Test-PathWithin {
    param(
        [Parameter(Mandatory)] [string]$Path,
        [Parameter(Mandatory)] [string]$Root
    )

    $resolvedPath = [IO.Path]::GetFullPath($Path)
    $resolvedRoot = [IO.Path]::GetFullPath($Root).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    return $resolvedPath.Equals($resolvedRoot, [StringComparison]::OrdinalIgnoreCase) -or
        $resolvedPath.StartsWith(
            $resolvedRoot + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase
        )
}

foreach ($sourceRoot in @(
        $repoRoot,
        (Join-Path $repoRoot "site"),
        (Join-Path $repoRoot "images"),
        (Join-Path $repoRoot "res")
    )) {
    if (Test-PathWithin -Path $resolvedOutput -Root $sourceRoot) {
        if ($sourceRoot -eq $repoRoot -and $resolvedOutput -ne $repoRoot) {
            continue
        }
        throw "Site output must not overwrite a source directory: $resolvedOutput"
    }
}

if (Test-Path -LiteralPath $resolvedOutput) {
    throw "Site output already exists; use a new empty path: $resolvedOutput"
}

$outputParent = Split-Path -Parent $resolvedOutput
if ([string]::IsNullOrWhiteSpace($outputParent)) {
    throw "Site output must have a parent directory: $resolvedOutput"
}
New-Item -ItemType Directory -Path $outputParent -Force | Out-Null

$stageName = ".nte-dps-site-stage-{0}" -f [Guid]::NewGuid().ToString("N")
$stageDirectory = [IO.Path]::GetFullPath((Join-Path $outputParent $stageName))
if ((Split-Path -Parent $stageDirectory) -ne [IO.Path]::GetFullPath($outputParent)) {
    throw "Site staging directory escaped its intended parent: $stageDirectory"
}

$assetCount = 0
try {
    New-Item -ItemType Directory -Path $stageDirectory | Out-Null

    foreach ($staticFile in @("index.html", "robots.txt", "sitemap.xml")) {
        $source = Join-Path $repoRoot (Join-Path "site" $staticFile)
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Site source file is missing: $source"
        }
        [IO.File]::Copy($source, (Join-Path $stageDirectory $staticFile), $false)
    }

    $imagesRoot = Join-Path $repoRoot "images"
    $assetRoot = Join-Path $stageDirectory "assets/img"
    $destinations = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    foreach ($source in Get-ChildItem -LiteralPath $imagesRoot -File -Recurse) {
        $relativePath = [IO.Path]::GetRelativePath($imagesRoot, $source.FullName)
        $destination = Join-Path $assetRoot $relativePath
        if (-not $destinations.Add([IO.Path]::GetFullPath($destination))) {
            throw "Two canonical site assets map to the same destination: $relativePath"
        }
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        [IO.File]::Copy($source.FullName, $destination, $false)
        $assetCount++
    }

    $iconSource = Join-Path $repoRoot "res/icons/app-icon.png"
    $iconDestination = Join-Path $assetRoot "app-icon.png"
    if (-not (Test-Path -LiteralPath $iconSource -PathType Leaf)) {
        throw "Canonical site icon is missing: $iconSource"
    }
    if (-not $destinations.Add([IO.Path]::GetFullPath($iconDestination))) {
        throw "Canonical site icon collides with another asset: $iconDestination"
    }
    New-Item -ItemType Directory -Path (Split-Path -Parent $iconDestination) -Force | Out-Null
    [IO.File]::Copy($iconSource, $iconDestination, $false)
    $assetCount++

    $html = Get-Content -LiteralPath (Join-Path $stageDirectory "index.html") -Raw
    $localReferences = [regex]::Matches($html, '(?:src|href)="([^"]+)"') |
        ForEach-Object { $_.Groups[1].Value } |
        Where-Object {
            $_ -notmatch '^(?:https?:|mailto:|data:|#)' -and
            -not [string]::IsNullOrWhiteSpace($_)
        } |
        Sort-Object -Unique
    foreach ($reference in $localReferences) {
        $relativeReference = ($reference -split '[?#]', 2)[0]
        $referencedPath = [IO.Path]::GetFullPath((Join-Path $stageDirectory $relativeReference))
        if (-not (Test-PathWithin -Path $referencedPath -Root $stageDirectory)) {
            throw "Site reference escapes the output directory: $reference"
        }
        if (-not (Test-Path -LiteralPath $referencedPath -PathType Leaf)) {
            throw "Site reference was not produced by the build: $reference"
        }
    }

    Move-Item -LiteralPath $stageDirectory -Destination $resolvedOutput
} catch {
    $stageParent = Split-Path -Parent $stageDirectory
    $stageLeaf = Split-Path -Leaf $stageDirectory
    if ($stageParent -eq [IO.Path]::GetFullPath($outputParent) -and
        $stageLeaf.StartsWith(".nte-dps-site-stage-", [StringComparison]::Ordinal) -and
        (Test-Path -LiteralPath $stageDirectory)) {
        Remove-Item -LiteralPath $stageDirectory -Recurse -Force
    }
    throw
}

$fileCount = @(Get-ChildItem -LiteralPath $resolvedOutput -File -Recurse).Count
Write-Output ("SITE-BUILD: OK output={0} files={1} assets={2}" -f $resolvedOutput, $fileCount, $assetCount)
