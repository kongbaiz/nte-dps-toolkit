param(
    [string]$Root = 'D:\NTE_DPS_TOOL'
)
$ErrorActionPreference = 'Stop'
$artifactRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$files = @(
    'src/core/live_capture.rs',
    'src-tauri/src/windows/island.rs',
    'src-tauri/src/channels/main_dps.rs',
    'src-tauri/src/commands/history.rs',
    'src-tauri/src/commands/encrypted_ini.rs'
)
foreach ($file in $files) {
    $source = Join-Path (Join-Path $artifactRoot 'originals') $file
    $destination = Join-Path $Root $file
    if (-not (Test-Path -LiteralPath $source)) {
        throw "Missing rollback source: $source"
    }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination -Force
    if ((Get-FileHash -Algorithm SHA256 $source).Hash -ne (Get-FileHash -Algorithm SHA256 $destination).Hash) {
        throw "Rollback hash mismatch: $file"
    }
    Write-Output "restored $file"
}
Write-Output 'rollback verified'
