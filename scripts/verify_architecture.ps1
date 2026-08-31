[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

function Assert-Policy {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Invoke-CargoLines {
    param([string[]]$Arguments)
    $lines = @(& cargo @Arguments)
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    return $lines
}

function Assert-TreeExcludes {
    param([string[]]$Tree, [string]$Pattern, [string]$Message)
    $matches = @($Tree | Select-String -Pattern $Pattern)
    Assert-Policy ($matches.Count -eq 0) "$Message`n$($matches -join "`n")"
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repositoryRoot
try {
    $metadata = Invoke-CargoLines @("metadata", "--format-version", "1", "--no-deps") |
        ConvertFrom-Json
    $packages = @($metadata.packages | Where-Object name -eq "nte-dps-tool")
    Assert-Policy ($packages.Count -eq 1) `
        "Cargo metadata must contain exactly one shared nte-dps-tool package"
    $package = $packages[0]

    $featureNames = @($package.features.PSObject.Properties.Name)
    Assert-Policy ($featureNames -contains "desktop") "Cargo feature 'desktop' is required"
    Assert-Policy ($featureNames -contains "cli") "Cargo feature 'cli' is required"
    Assert-Policy ($featureNames -notcontains "gui") "Legacy Cargo feature 'gui' must stay removed"
    Assert-Policy (@($package.features.default) -join "," -eq "desktop") `
        "The default Cargo feature set must remain exactly ['desktop']"

    $directDependencyNames = @(
        $package.dependencies |
            Where-Object { $null -eq $_.kind } |
            ForEach-Object name
    )
    foreach ($name in @("eframe", "egui_material_icons", "rfd", "raw-window-handle")) {
        Assert-Policy ($directDependencyNames -notcontains $name) `
            "Legacy direct dependency '$name' must stay removed"
    }

    $legacyGuiTargets = @(
        $package.targets |
            Where-Object { $_.name -eq "nte-dps-tool" -and $_.kind -contains "bin" }
    )
    Assert-Policy ($legacyGuiTargets.Count -eq 0) `
        "The shared crate must not restore the legacy nte-dps-tool GUI binary"

    $updaterTargets = @(
        $package.targets |
            Where-Object { $_.name -eq "nte-updater" -and $_.kind -contains "bin" }
    )
    Assert-Policy ($updaterTargets.Count -eq 1) "Expected exactly one nte-updater binary target"
    Assert-Policy (@($updaterTargets[0].'required-features') -join "," -eq "desktop") `
        "The updater binary must require exactly feature 'desktop'"

    $cliTree = Invoke-CargoLines @("tree", "-e", "normal", "--no-default-features", "--features", "cli")
    Assert-TreeExcludes $cliTree `
        '(^|[\s│├└─])(?:tauri|wry|webview2-com|eframe|egui(?:-[A-Za-z0-9_-]+)?|wgpu(?:-[A-Za-z0-9_-]+)?|rfd|raw-window-handle) v' `
        "CLI dependency tree contains desktop UI crates:"

    $desktopTree = Invoke-CargoLines @("tree", "-e", "normal", "--no-default-features", "--features", "desktop")
    Assert-TreeExcludes $desktopTree `
        '(^|[\s│├└─])(?:eframe|egui(?:-[A-Za-z0-9_-]+)?|wgpu(?:-[A-Za-z0-9_-]+)?|rfd) v' `
        "Shared desktop dependency tree contains legacy UI crates:"

    $tauriTree = Invoke-CargoLines @("tree", "--manifest-path", "src-tauri/Cargo.toml", "-e", "normal")
    Assert-TreeExcludes $tauriTree `
        '(^|[\s│├└─])(?:eframe|egui(?:-[A-Za-z0-9_-]+)?|wgpu(?:-[A-Za-z0-9_-]+)?) v' `
        "Tauri dependency tree contains legacy UI crates:"

    $rfdReverseTree = Invoke-CargoLines @(
        "tree", "--manifest-path", "src-tauri/Cargo.toml", "-e", "normal", "-i", "rfd", "--prefix", "none"
    )
    $unexpectedRfdConsumers = @(
        $rfdReverseTree |
            Where-Object { $_ -match '^(?<name>[A-Za-z0-9_.-]+) v' } |
            ForEach-Object { $Matches.name } |
            Where-Object { $_ -notin @("rfd", "tauri-plugin-dialog", "nte-dps-tool-tauri") }
    )
    Assert-Policy ($unexpectedRfdConsumers.Count -eq 0) `
        "rfd must only be resolved through tauri-plugin-dialog: $($unexpectedRfdConsumers -join ', ')"

    Assert-Policy (-not (Test-Path -LiteralPath "src/app")) "Legacy src/app directory must stay removed"
    Assert-Policy (-not (Test-Path -LiteralPath "vendor/egui-winit-0.34.3")) `
        "Vendored egui-winit must stay removed"

    Write-Output "Architecture policy passed: Tauri is the sole desktop UI and CLI remains UI-free."
}
finally {
    Pop-Location
}
