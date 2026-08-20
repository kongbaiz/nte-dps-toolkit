[CmdletBinding()]
param(
    [switch]$SelfTestOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$forbiddenCliDependencyPattern =
    '(^|[\s│├└─])(?:tauri|wry|webview2-com|eframe|egui(?:-[A-Za-z0-9_-]+)?|wgpu(?:-[A-Za-z0-9_-]+)?|rfd|raw-window-handle) v'
$forbiddenLegacyUiDependencyPattern =
    '(^|[\s│├└─])(?:eframe|egui(?:-[A-Za-z0-9_-]+)?|wgpu(?:-[A-Za-z0-9_-]+)?|rfd) v'
$approvedTauriDialogAncestor = "tauri-plugin-dialog"
$forbiddenCliRealNames = @(
    "tauri", "wry", "webview2-com", "eframe", "egui", "wgpu", "rfd",
    "raw-window-handle"
)
$forbiddenLegacyUiRealNames = @("eframe", "egui", "wgpu", "rfd")

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

function Find-ForbiddenDependency {
    param(
        [Parameter(Mandatory)]
        [string[]]$TreeLines,
        [Parameter(Mandatory)]
        [string]$Pattern
    )

    @($TreeLines | Select-String -Pattern $Pattern)
}

function Test-ForbiddenLockedName {
    param(
        [Parameter(Mandatory)]
        [string]$Name,
        [Parameter(Mandatory)]
        [string[]]$ForbiddenNames
    )

    foreach ($forbidden in $ForbiddenNames) {
        if (
            $Name -eq $forbidden -or
            $Name.StartsWith("$forbidden-") -or
            $Name.StartsWith("${forbidden}_")
        ) {
            return $true
        }
    }
    return $false
}

function ConvertFrom-CargoTreeLine {
    param(
        [Parameter(Mandatory)]
        [string]$Line
    )

    $connector = [regex]::Match($Line, '[├└]── ')
    if ($connector.Success) {
        $depth = [int]($connector.Index / 4) + 1
        $nodeText = $Line.Substring($connector.Index + $connector.Length)
    }
    else {
        $depth = 0
        $nodeText = $Line
    }

    if ($nodeText -notmatch '^(?<name>[A-Za-z0-9_.-]+) v') {
        return $null
    }

    [pscustomobject]@{
        Depth = $depth
        Name = $Matches.name
        Line = $Line
    }
}

function Find-ForbiddenTauriDependency {
    param(
        [Parameter(Mandatory)]
        [string[]]$TreeLines
    )

    $ancestors = @()
    $violations = @()
    foreach ($line in $TreeLines) {
        $node = ConvertFrom-CargoTreeLine $line
        if ($null -eq $node) {
            continue
        }

        $ancestors = @($ancestors | Select-Object -First $node.Depth)
        $isForbidden = Test-ForbiddenLockedName $node.Name $script:forbiddenLegacyUiRealNames
        $isApprovedDialogDependency =
            $node.Name -eq "rfd" -and
            $ancestors -contains $script:approvedTauriDialogAncestor
        if ($isForbidden -and -not $isApprovedDialogDependency) {
            $violations += $node.Line
        }
        $ancestors += $node.Name
    }
    return $violations
}

function Find-ForbiddenLockedNames {
    param(
        [Parameter(Mandatory)]
        [string]$Lockfile,
        [Parameter(Mandatory)]
        [string[]]$ForbiddenNames
    )

    if (-not (Test-Path -LiteralPath $Lockfile)) {
        return @()
    }
    return @(
        Get-Content -LiteralPath $Lockfile |
            Select-String -Pattern '^name = "(.+)"' |
            ForEach-Object { $_.Matches[0].Groups[1].Value } |
            Where-Object { Test-ForbiddenLockedName $_ $ForbiddenNames }
    )
}

function Test-PolicyHelpers {
    $cleanTree = @(
        "nte-dps-tool v0.0.0",
        "serde v1.0.0"
    )
    Assert-Policy `
        (@(Find-ForbiddenDependency $cleanTree $script:forbiddenCliDependencyPattern).Count -eq 0) `
        "Architecture policy self-test rejected a clean dependency tree"

    $forbiddenTree = @(
        "nte-dps-tool v0.0.0",
        "egui-winit v0.0.0",
        "tauri v0.0.0"
    )
    Assert-Policy `
        (@(Find-ForbiddenDependency $forbiddenTree $script:forbiddenCliDependencyPattern).Count -eq 2) `
        "Architecture policy self-test missed a forbidden dependency"

    $cliRfdTree = @(
        "nte-dps-tool v0.0.0",
        "└── rfd v0.0.0"
    )
    Assert-Policy `
        (@(Find-ForbiddenDependency $cliRfdTree $script:forbiddenCliDependencyPattern).Count -eq 1) `
        "Architecture policy self-test allowed rfd in the root/CLI dependency tree"

    $approvedTauriTree = @(
        "nte-dps-tool-desktop v0.0.0",
        "└── tauri-plugin-dialog v0.0.0",
        "    └── rfd v0.0.0"
    )
    Assert-Policy `
        (@(Find-ForbiddenTauriDependency $approvedTauriTree).Count -eq 0) `
        "Architecture policy self-test rejected tauri-plugin-dialog's rfd dependency"

    $directTauriRfdTree = @(
        "nte-dps-tool-desktop v0.0.0",
        "└── rfd v0.0.0"
    )
    Assert-Policy `
        (@(Find-ForbiddenTauriDependency $directTauriRfdTree).Count -eq 1) `
        "Architecture policy self-test allowed a direct Tauri rfd dependency"

    $unapprovedTauriRfdTree = @(
        "nte-dps-tool-desktop v0.0.0",
        "└── unrelated-dialog-wrapper v0.0.0",
        "    └── rfd v0.0.0"
    )
    Assert-Policy `
        (@(Find-ForbiddenTauriDependency $unapprovedTauriRfdTree).Count -eq 1) `
        "Architecture policy self-test allowed rfd outside tauri-plugin-dialog"

    $overbroadTauriExemptionTree = @(
        "nte-dps-tool-desktop v0.0.0",
        "└── tauri-plugin-dialog v0.0.0",
        "    └── egui v0.0.0"
    )
    Assert-Policy `
        (@(Find-ForbiddenTauriDependency $overbroadTauriExemptionTree).Count -eq 1) `
        "Architecture policy self-test exempted a non-rfd legacy UI dependency"

    Assert-Policy `
        (Test-ForbiddenLockedName "tauri-utils" $script:forbiddenCliRealNames) `
        "Forbidden locked-name helper must match prefixed real names"
    Assert-Policy `
        (Test-ForbiddenLockedName "rfd" $script:forbiddenCliRealNames) `
        "Forbidden locked-name helper must continue rejecting rfd in the root lockfile"
    Assert-Policy `
        (-not (Test-ForbiddenLockedName "serde" $script:forbiddenCliRealNames)) `
        "Forbidden locked-name helper must reject unrelated names"
    Assert-Policy `
        (-not (Test-ForbiddenLockedName "egui2" $script:forbiddenCliRealNames)) `
        "Forbidden locked-name helper must not match suffix-only names"
}

Test-PolicyHelpers
if ($SelfTestOnly) {
    Write-Output "Architecture policy helper tests passed."
    exit 0
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repositoryRoot
try {
    $metadata = (& cargo metadata --format-version 1 --no-deps | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
    $packages = @($metadata.packages | Where-Object { $_.name -eq "nte-dps-tool" })
    Assert-Policy ($packages.Count -eq 1) "Cargo metadata must contain exactly one shared nte-dps-tool package"
    $package = $packages[0]

    $featureNames = @($package.features.PSObject.Properties.Name)
    Assert-Policy ($featureNames -contains "desktop") "Cargo feature 'desktop' is required"
    Assert-Policy ($featureNames -contains "cli") "Cargo feature 'cli' is required"
    Assert-Policy ($featureNames -notcontains "gui") "Legacy Cargo feature 'gui' must stay removed"

    $defaultFeatures = @($package.features.default)
    Assert-Policy `
        (($defaultFeatures.Count -eq 1) -and ($defaultFeatures[0] -eq "desktop")) `
        "The default Cargo feature set must remain exactly ['desktop']"

    foreach ($dependencyName in @(
        "eframe",
        "egui_material_icons",
        "rfd",
        "raw-window-handle"
    )) {
        $dependencies = @(
            $package.dependencies |
                Where-Object { $_.name -eq $dependencyName -and $null -eq $_.kind }
        )
        Assert-Policy `
            ($dependencies.Count -eq 0) `
            "Legacy direct dependency '$dependencyName' must stay removed"
    }

    $legacyGuiTargets = @(
        $package.targets |
            Where-Object { $_.name -eq "nte-dps-tool" -and $_.kind -contains "bin" }
    )
    Assert-Policy `
        ($legacyGuiTargets.Count -eq 0) `
        "The shared crate must not restore the legacy nte-dps-tool GUI binary"

    $updaterTargets = @(
        $package.targets |
            Where-Object { $_.name -eq "nte-updater" -and $_.kind -contains "bin" }
    )
    Assert-Policy ($updaterTargets.Count -eq 1) "Expected exactly one nte-updater binary target"
    $updaterRequiredFeatures = @($updaterTargets[0].'required-features')
    Assert-Policy `
        (($updaterRequiredFeatures.Count -eq 1) -and ($updaterRequiredFeatures[0] -eq "desktop")) `
        "The updater binary must require exactly feature 'desktop'"

    $cliTree = @(& cargo tree -e normal --no-default-features --features cli)
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
    $forbiddenCli = @(Find-ForbiddenDependency $cliTree $forbiddenCliDependencyPattern)
    Assert-Policy `
        ($forbiddenCli.Count -eq 0) `
        "CLI dependency tree contains desktop UI crates:`n$($forbiddenCli -join "`n")"

    $desktopTree = @(& cargo tree -e normal --no-default-features --features desktop)
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
    $forbiddenDesktop = @(Find-ForbiddenDependency $desktopTree $forbiddenLegacyUiDependencyPattern)
    Assert-Policy `
        ($forbiddenDesktop.Count -eq 0) `
        "Shared desktop dependency tree contains legacy UI crates:`n$($forbiddenDesktop -join "`n")"

    $tauriTree = @(& cargo tree --manifest-path src-tauri/Cargo.toml -e normal)
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
    $forbiddenTauri = @(Find-ForbiddenTauriDependency $tauriTree)
    Assert-Policy `
        ($forbiddenTauri.Count -eq 0) `
        "Tauri dependency tree contains legacy UI crates:`n$($forbiddenTauri -join "`n")"

    $forbiddenLockedCli = @(
        Find-ForbiddenLockedNames (Join-Path $repositoryRoot "Cargo.lock") $forbiddenCliRealNames
    )
    Assert-Policy `
        ($forbiddenLockedCli.Count -eq 0) `
        "Root crate lockfile must not resolve desktop UI crates: $($forbiddenLockedCli -join ', ')"

    $forbiddenLockedTauri = @(
        Find-ForbiddenLockedNames `
            (Join-Path $repositoryRoot "src-tauri\Cargo.lock") `
            @("eframe", "egui", "wgpu")
    )
    Assert-Policy `
        ($forbiddenLockedTauri.Count -eq 0) `
        "Tauri lockfile must not resolve legacy UI crates: $($forbiddenLockedTauri -join ', ')"

    $lockedTauriRfd = @(
        Find-ForbiddenLockedNames `
            (Join-Path $repositoryRoot "src-tauri\Cargo.lock") `
            @("rfd")
    )
    $resolvedTauriRfd = @($tauriTree | Where-Object { $_ -match '(^|[\s│├└─])rfd v' })
    Assert-Policy `
        ($lockedTauriRfd.Count -eq 0 -or $resolvedTauriRfd.Count -gt 0) `
        "Tauri lockfile contains stale rfd without an approved resolved dependency path"

    Assert-Policy (-not (Test-Path -LiteralPath "src/app")) "Legacy src/app directory must stay removed"
    Assert-Policy (-not (Test-Path -LiteralPath "vendor/egui-winit-0.34.3")) "Vendored egui-winit must stay removed"

    Write-Output "Architecture policy passed: Tauri is the sole desktop UI and CLI remains UI-free."
}
finally {
    Pop-Location
}
