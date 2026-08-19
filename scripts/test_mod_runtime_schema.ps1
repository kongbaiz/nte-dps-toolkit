$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
& (Join-Path $PSScriptRoot "generate_mod_runtime_schema.ps1") -Check

$rust = Get-Content -LiteralPath (Join-Path $repoRoot "src\storage\mod_scripts.rs") -Raw
$native = Get-Content -LiteralPath (Join-Path $repoRoot "native\nte-mods-plugin\src\mod_runtime.cpp") -Raw
if ($rust -notmatch 'include!\("mod_runtime_schema\.generated\.rs"\)' -or
    $rust -notmatch 'MOD_CAPABILITIES\s*\.iter\(\)' -or
    $rust -notmatch 'MOD_IPC_SERVICES\s*\.iter\(\)') {
    throw "Rust Mod validation is not consuming the generated runtime schema."
}
if ($native -notmatch '#include "mod_runtime_schema\.generated\.hpp"' -or
    $native -notmatch 'schema::CAPABILITIES' -or
    $native -notmatch 'schema::SERVICES') {
    throw "Native Mod runtime is not consuming the generated runtime schema."
}

Write-Output "MOD-RUNTIME-SCHEMA-CONSUMERS: PASS"
