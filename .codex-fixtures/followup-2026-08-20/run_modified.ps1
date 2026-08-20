param([string]$Repository = 'D:\NTE_DPS_TOOL')
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Continue'
$env:CARGO_TERM_COLOR = 'never'
$env:NO_COLOR = '1'
$env:CI = 'true'
$logPath = Join-Path $PSScriptRoot 'MODIFIED.log'
$summaryPath = Join-Path $PSScriptRoot 'MODIFIED_SUMMARY.txt'
$log = [Collections.Generic.List[string]]::new()
$summary = [Collections.Generic.List[string]]::new()
$failed = $false

function Invoke-VerificationStep {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$Command,
        [Parameter(Mandatory)][scriptblock]$Action
    )
    $started = Get-Date
    $script:log.Add("===== $Name =====")
    $script:log.Add("COMMAND: $Command")
    $global:LASTEXITCODE = 0
    $lines = @(& $Action 2>&1 | ForEach-Object { $_.ToString() })
    $code = $global:LASTEXITCODE
    foreach ($line in $lines) { $script:log.Add($line) }
    $script:log.Add("EXIT: $code")
    $script:log.Add('')
    $elapsed = [math]::Round(((Get-Date) - $started).TotalSeconds, 2)
    $script:summary.Add("$Name`tEXIT=$code`tSECONDS=$elapsed")
    if ($code -ne 0) { $script:failed = $true }
    [IO.File]::WriteAllLines($logPath, $script:log, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllLines($summaryPath, $script:summary, [Text.UTF8Encoding]::new($false))
}

Push-Location $Repository
try {
    Invoke-VerificationStep 'root fmt' 'cargo fmt --check' { cargo fmt --check }
    Invoke-VerificationStep 'root check' 'cargo check --locked' { cargo check --locked }
    Invoke-VerificationStep 'root test' 'cargo test --locked' { cargo test --locked }
    Invoke-VerificationStep 'root clippy desktop' 'cargo clippy --locked --all-targets --features desktop -- -D warnings' { cargo clippy --locked --all-targets --features desktop -- -D warnings }
    Invoke-VerificationStep 'CLI check' 'cargo check --locked --bin nte-core --no-default-features --features cli' { cargo check --locked --bin nte-core --no-default-features --features cli }
    Invoke-VerificationStep 'CLI test' 'cargo test --locked --no-default-features --features cli' { cargo test --locked --no-default-features --features cli }
    Invoke-VerificationStep 'CLI clippy' 'cargo clippy --locked --all-targets --no-default-features --features cli -- -D warnings' { cargo clippy --locked --all-targets --no-default-features --features cli -- -D warnings }
    Invoke-VerificationStep 'Tauri fmt' 'cargo fmt --check --manifest-path src-tauri/Cargo.toml' { cargo fmt --check --manifest-path src-tauri/Cargo.toml }
    Invoke-VerificationStep 'Tauri check' 'cargo check --locked --manifest-path src-tauri/Cargo.toml' { cargo check --locked --manifest-path src-tauri/Cargo.toml }
    Invoke-VerificationStep 'Tauri test' 'cargo test --locked --manifest-path src-tauri/Cargo.toml' { cargo test --locked --manifest-path src-tauri/Cargo.toml }
    Invoke-VerificationStep 'Tauri clippy' 'cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings' { cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings }
    Invoke-VerificationStep 'frontend lint' 'pnpm --dir frontend lint' { pnpm --dir frontend lint }
    Invoke-VerificationStep 'frontend typecheck' 'pnpm --dir frontend typecheck' { pnpm --dir frontend typecheck }
    Invoke-VerificationStep 'frontend test' 'pnpm --dir frontend test' { pnpm --dir frontend test }
    Invoke-VerificationStep 'frontend build' 'pnpm --dir frontend build' { pnpm --dir frontend build }
    Invoke-VerificationStep 'architecture gate' 'pwsh -NoProfile -File scripts/verify_architecture.ps1' { pwsh -NoProfile -File scripts/verify_architecture.ps1 }
    Invoke-VerificationStep 'runtime safety gate' 'pwsh -NoProfile -File scripts/verify_runtime_safety.ps1' { pwsh -NoProfile -File scripts/verify_runtime_safety.ps1 }
    Invoke-VerificationStep 'site build gate' 'pwsh -NoProfile -File scripts/test_site_build.ps1' { pwsh -NoProfile -File scripts/test_site_build.ps1 }
    Invoke-VerificationStep 'runtime schema gate' 'pwsh -NoProfile -File scripts/test_mod_runtime_schema.ps1' { pwsh -NoProfile -File scripts/test_mod_runtime_schema.ps1 }
    Invoke-VerificationStep 'contract version parity' 'pwsh -NoProfile -File scripts/test_contract_version_parity.ps1' { pwsh -NoProfile -File scripts/test_contract_version_parity.ps1 }
    Invoke-VerificationStep 'i18n source coverage' 'pwsh -NoProfile -File scripts/test_i18n_source_coverage.ps1' { pwsh -NoProfile -File scripts/test_i18n_source_coverage.ps1 }
    Invoke-VerificationStep 'CI PowerShell fail-fast' 'pwsh -NoProfile -File scripts/test_ci_powershell_fail_fast.ps1' { pwsh -NoProfile -File scripts/test_ci_powershell_fail_fast.ps1 }
    Invoke-VerificationStep 'native Release clean' "& 'C:\Program Files\Microsoft Visual Studio\18\Community\MSBuild\Current\Bin\MSBuild.exe' .\native\nte-mods-plugin\nte-mods-plugin.sln /m /t:Clean /p:Configuration=Release /p:Platform=x64 /nologo /verbosity:minimal" { & 'C:\Program Files\Microsoft Visual Studio\18\Community\MSBuild\Current\Bin\MSBuild.exe' .\native\nte-mods-plugin\nte-mods-plugin.sln /m /t:Clean /p:Configuration=Release /p:Platform=x64 /nologo /verbosity:minimal }
    Invoke-VerificationStep 'native Release build' "& 'C:\Program Files\Microsoft Visual Studio\18\Community\MSBuild\Current\Bin\MSBuild.exe' .\native\nte-mods-plugin\nte-mods-plugin.sln /m /t:Build /p:Configuration=Release /p:Platform=x64 /nologo /verbosity:minimal" { & 'C:\Program Files\Microsoft Visual Studio\18\Community\MSBuild\Current\Bin\MSBuild.exe' .\native\nte-mods-plugin\nte-mods-plugin.sln /m /t:Build /p:Configuration=Release /p:Platform=x64 /nologo /verbosity:minimal }
    Invoke-VerificationStep 'native lifecycle source' 'pwsh -NoProfile -File native/nte-mods-plugin/tests/native_lifecycle_source_tests.ps1' { pwsh -NoProfile -File native/nte-mods-plugin/tests/native_lifecycle_source_tests.ps1 }
    Invoke-VerificationStep 'native IPC source' 'pwsh -NoProfile -File native/nte-mods-plugin/tests/ipc_transport_security_source_tests.ps1' { pwsh -NoProfile -File native/nte-mods-plugin/tests/ipc_transport_security_source_tests.ps1 }
    Invoke-VerificationStep 'native signature policy' 'pwsh -NoProfile -File scripts/test_native_signature_policy.ps1' { pwsh -NoProfile -File scripts/test_native_signature_policy.ps1 }
    Invoke-VerificationStep 'native shadow vtable' 'pwsh -NoProfile -File scripts/test_native_shadow_vtable_hook.ps1' { pwsh -NoProfile -File scripts/test_native_shadow_vtable_hook.ps1 }
    Invoke-VerificationStep 'native proxy forwarding' 'pwsh -NoProfile -File native/nte-mods-plugin/tests/proxy_forwarding_runtime_tests.ps1' { pwsh -NoProfile -File native/nte-mods-plugin/tests/proxy_forwarding_runtime_tests.ps1 }
    Invoke-VerificationStep 'native remote bootstrap' 'cargo test --locked --features desktop remote_bootstrap_invokes_loaded_export -- --ignored --nocapture' { cargo test --locked --features desktop remote_bootstrap_invokes_loaded_export -- --ignored --nocapture }
    # The 500001-hit ignored History regression was already executed against
    # this working tree (PASS, 1579.88s). Keep the standard Hardened matrix
    # repeat bounded; its exact output is retained in the final verification.
    Invoke-VerificationStep 'long combat retention' 'cargo test --locked --lib retains_every_hit_and_lifetime_total -- --nocapture' { cargo test --locked --lib retains_every_hit_and_lifetime_total -- --nocapture }
    Invoke-VerificationStep 'root object hygiene' '$rootObjs=@(Get-ChildItem -LiteralPath . -File -Filter *.obj -Force); Write-Output "ROOT_OBJ_ARTIFACTS=$($rootObjs.Count)"; if($rootObjs.Count -gt 0){exit 1}' { $rootObjs=@(Get-ChildItem -LiteralPath . -File -Filter *.obj -Force); Write-Output "ROOT_OBJ_ARTIFACTS=$($rootObjs.Count)"; if($rootObjs.Count -gt 0){$global:LASTEXITCODE=1}else{$global:LASTEXITCODE=0} }
}
finally {
    Pop-Location
}
if ($failed) { exit 1 }
exit 0
