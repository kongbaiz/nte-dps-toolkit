$ErrorActionPreference = 'Continue'
$repo = 'D:\NTE_DPS_TOOL'
$log = 'D:\NTE_DPS_TOOL\.codex-fixtures\review-2026-08-19\MODIFIED.log'
$summary = 'D:\NTE_DPS_TOOL\.codex-fixtures\review-2026-08-19\MODIFIED_SUMMARY.txt'
$pnpm = 'C:\Users\ZWB\.cache\codex-runtimes\codex-primary-runtime\dependencies\bin\fallback\pnpm.cmd'
$env:CI = 'true'
$msbuildCandidates = @(
  'C:\Program Files\Microsoft Visual Studio\18\Community\MSBuild\Current\Bin\MSBuild.exe',
  'C:\Program Files\Microsoft Visual Studio\2022\Community\MSBuild\Current\Bin\MSBuild.exe',
  'C:\Program Files\Microsoft Visual Studio\2022\Professional\MSBuild\Current\Bin\MSBuild.exe'
)
$msbuild = $msbuildCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
Set-Content -LiteralPath $log -Value "MODIFIED BRANCH: $(& git -C $repo branch --show-current)`nMODIFIED HEAD: $(& git -C $repo rev-parse HEAD)"
Set-Content -LiteralPath $summary -Value 'command|exit'

function Run([string]$label, [scriptblock]$action) {
  Add-Content -LiteralPath $log -Value ''
  Add-Content -LiteralPath $log -Value "=== COMMAND: $label ==="
  Write-Host ''
  Write-Host "=== COMMAND: $label ==="
  $global:LASTEXITCODE = 0
  & $action 2>&1 | Tee-Object -FilePath $log -Append
  $code = $LASTEXITCODE
  if ($null -eq $code) { $code = 0 }
  Add-Content -LiteralPath $log -Value "=== EXIT: $code ==="
  Add-Content -LiteralPath $summary -Value "$label|$code"
  Write-Host "=== EXIT: $code ==="
}

Set-Location $repo
Run 'git diff --check' { git diff --check }
Run 'cargo fmt --check' { cargo fmt --check }
Run 'cargo check --locked' { cargo check --locked }
Run 'cargo test --locked' { cargo test --locked }
Run 'cargo clippy --locked --all-targets --features desktop -- -D warnings' { cargo clippy --locked --all-targets --features desktop -- -D warnings }
Run 'cargo check --locked --bin nte-core --no-default-features --features cli' { cargo check --locked --bin nte-core --no-default-features --features cli }
Run 'cargo test --locked --no-default-features --features cli' { cargo test --locked --no-default-features --features cli }
Run 'cargo clippy --locked --all-targets --no-default-features --features cli -- -D warnings' { cargo clippy --locked --all-targets --no-default-features --features cli -- -D warnings }
Run 'cargo fmt --check --manifest-path src-tauri/Cargo.toml' { cargo fmt --check --manifest-path src-tauri/Cargo.toml }
Run 'cargo check --locked --manifest-path src-tauri/Cargo.toml' { cargo check --locked --manifest-path src-tauri/Cargo.toml }
Run 'cargo test --locked --manifest-path src-tauri/Cargo.toml' { cargo test --locked --manifest-path src-tauri/Cargo.toml }
Run 'cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings' { cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings }
Run 'CI=true pnpm --dir frontend lint' { & $pnpm --dir frontend lint }
Run 'CI=true pnpm --dir frontend typecheck' { & $pnpm --dir frontend typecheck }
Run 'CI=true pnpm --dir frontend test' { & $pnpm --dir frontend test }
Run 'CI=true pnpm --dir frontend build' { & $pnpm --dir frontend build }
Run 'pwsh -NoProfile -File scripts/verify_architecture.ps1' { pwsh -NoProfile -File scripts/verify_architecture.ps1 }
if (Test-Path -LiteralPath (Join-Path $repo 'scripts\verify_runtime_safety.ps1')) {
  Run 'pwsh -NoProfile -File scripts/verify_runtime_safety.ps1' { pwsh -NoProfile -File scripts/verify_runtime_safety.ps1 }
}
Run 'pwsh -NoProfile -File native/nte-mods-plugin/tests/native_lifecycle_source_tests.ps1' { pwsh -NoProfile -File native/nte-mods-plugin/tests/native_lifecycle_source_tests.ps1 }
Run 'pwsh -NoProfile -File native/nte-mod-loader/tests/manual_map_protocol_source_tests.ps1' { pwsh -NoProfile -File native/nte-mod-loader/tests/manual_map_protocol_source_tests.ps1 }

if ($msbuild) {
  $pluginLabel = '"' + $msbuild + '" native\nte-mods-plugin\nte-mods-plugin.vcxproj /t:Clean,Build /p:Configuration=Release /p:Platform=x64 /m /nologo /v:minimal'
  Run $pluginLabel {
    & $msbuild 'native\nte-mods-plugin\nte-mods-plugin.vcxproj' /t:Clean,Build /p:Configuration=Release /p:Platform=x64 /m /nologo /v:minimal
  }
  $loaderLabel = '"' + $msbuild + '" native\nte-mod-loader\nte-mod-loader.sln /t:Clean,Build /p:Configuration=Release /p:Platform=x64 /m /nologo /v:minimal'
  Run $loaderLabel {
    & $msbuild 'native\nte-mod-loader\nte-mod-loader.sln' /t:Clean,Build /p:Configuration=Release /p:Platform=x64 /m /nologo /v:minimal
  }
  Run 'native\nte-mod-loader\bin\Release\nte-mod-loader-tests.exe' { & 'native\nte-mod-loader\bin\Release\nte-mod-loader-tests.exe' }
} else {
  Add-Content -LiteralPath $summary -Value 'MSBuild Release x64|NOT_FOUND'
}

Get-Content -LiteralPath $summary
