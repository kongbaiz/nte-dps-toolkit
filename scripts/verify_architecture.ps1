[CmdletBinding()]
param(
    [switch]$SelfTestOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$forbiddenCliDependencyPattern =
    '(^|[\s│├└─])(?:eframe|egui(?:-[A-Za-z0-9_-]+)?|wgpu(?:-[A-Za-z0-9_-]+)?|rfd|raw-window-handle) v'

function Assert-Policy {
    param(
        [Parameter(Mandatory)]
        [bool]$Condition,
        [Parameter(Mandatory)]
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Find-ForbiddenCliDependency {
    param(
        [Parameter(Mandatory)]
        [string[]]$TreeLines
    )

    @($TreeLines | Select-String -Pattern $script:forbiddenCliDependencyPattern)
}

function Test-PolicyHelpers {
    $cleanTree = @(
        "nte-dps-tool v0.0.0",
        "serde v1.0.0"
    )
    $cleanMatches = @(Find-ForbiddenCliDependency $cleanTree)
    Assert-Policy `
        ($cleanMatches.Count -eq 0) `
        "Architecture policy self-test rejected a clean dependency tree"

    $forbiddenTree = @(
        "nte-dps-tool v0.0.0",
        "egui-winit v0.0.0",
        "wgpu-core v0.0.0"
    )
    $forbiddenMatches = @(Find-ForbiddenCliDependency $forbiddenTree)
    Assert-Policy `
        ($forbiddenMatches.Count -eq 2) `
        "Architecture policy self-test missed a forbidden GUI dependency"
}

Test-PolicyHelpers
if ($SelfTestOnly) {
    Write-Output "Architecture policy helper tests passed."
    exit 0
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repositoryRoot
try {
    $metadataJson = & cargo metadata --format-version 1 --no-deps
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
    $metadata = $metadataJson | ConvertFrom-Json
    $packages = @($metadata.packages | Where-Object { $_.name -eq "nte-dps-tool" })
    Assert-Policy ($packages.Count -eq 1) "Cargo metadata must contain exactly one nte-dps-tool package"
    $package = $packages[0]

    $featureNames = @($package.features.PSObject.Properties.Name)
    Assert-Policy ($featureNames -contains "gui") "Cargo feature 'gui' is required"
    Assert-Policy ($featureNames -contains "cli") "Cargo feature 'cli' is required"

    $defaultFeatures = @($package.features.default)
    Assert-Policy `
        (($defaultFeatures.Count -eq 1) -and ($defaultFeatures[0] -eq "gui")) `
        "The default Cargo feature set must remain exactly ['gui']"

    $guiFeatures = @($package.features.gui)
    foreach ($dependencyName in @(
        "eframe",
        "egui_material_icons",
        "image",
        "raw-window-handle",
        "rfd"
    )) {
        $dependencies = @(
            $package.dependencies |
                Where-Object { $_.name -eq $dependencyName -and $null -eq $_.kind }
        )
        Assert-Policy `
            ($dependencies.Count -eq 1) `
            "Expected one direct dependency entry for '$dependencyName'"
        Assert-Policy `
            ([bool]$dependencies[0].optional) `
            "GUI dependency '$dependencyName' must remain optional"
        Assert-Policy `
            ($guiFeatures -contains "dep:$dependencyName") `
            "GUI dependency '$dependencyName' must be enabled only through feature 'gui'"
    }

    $guiTargets = @(
        $package.targets |
            Where-Object { $_.name -eq "nte-dps-tool" -and $_.kind -contains "bin" }
    )
    Assert-Policy ($guiTargets.Count -eq 1) "Expected exactly one nte-dps-tool binary target"
    $guiRequiredFeatures = @($guiTargets[0].'required-features')
    Assert-Policy `
        (($guiRequiredFeatures.Count -eq 1) -and ($guiRequiredFeatures[0] -eq "gui")) `
        "The nte-dps-tool binary must require exactly feature 'gui'"

    $cliTargets = @(
        $package.targets |
            Where-Object { $_.name -eq "nte-core" -and $_.kind -contains "bin" }
    )
    Assert-Policy ($cliTargets.Count -eq 1) "Expected exactly one nte-core binary target"
    $cliRequiredFeatures = @($cliTargets[0].'required-features')
    Assert-Policy `
        (($cliRequiredFeatures.Count -eq 1) -and ($cliRequiredFeatures[0] -eq "cli")) `
        "The nte-core binary must require exactly feature 'cli'"

    $cliTree = @(& cargo tree -e normal --no-default-features --features cli)
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
    $forbiddenDependencies = @(Find-ForbiddenCliDependency $cliTree)
    if ($forbiddenDependencies.Count -ne 0) {
        throw "CLI dependency tree contains GUI crates:`n$($forbiddenDependencies -join "`n")"
    }

    Write-Output "Architecture policy passed: Cargo features, binary gates, and CLI dependency isolation are intact."
}
finally {
    Pop-Location
}
