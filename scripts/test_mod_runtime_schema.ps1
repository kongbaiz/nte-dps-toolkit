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

$generator = Join-Path $PSScriptRoot "generate_mod_runtime_schema.ps1"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) (
    "nte-mod-runtime-schema-{0}" -f [Guid]::NewGuid().ToString("N")
)
$utf8 = [Text.UTF8Encoding]::new($false)
try {
    & $generator -OutputRoot $tempRoot

    $generated = @(
        "src/storage/mod_runtime_schema.generated.rs",
        "native/nte-mods-plugin/src/mod_runtime_schema.generated.hpp"
    )
    foreach ($relative in $generated) {
        $path = Join-Path $tempRoot $relative
        $lf = [IO.File]::ReadAllText($path).Replace("`r`n", "`n").Replace("`r", "`n")
        [IO.File]::WriteAllText($path, $lf.Replace("`n", "`r`n"), $utf8)
    }
    & $generator -Check -OutputRoot $tempRoot

    $rustPath = Join-Path $tempRoot $generated[0]
    [IO.File]::AppendAllText($rustPath, "// semantic drift`r`n", $utf8)
    $driftRejected = $false
    try {
        & $generator -Check -OutputRoot $tempRoot
    } catch {
        if ($_.Exception.Message -notlike "Generated Mod runtime schema is stale:*") {
            throw
        }
        $driftRejected = $true
    }
    if (-not $driftRejected) {
        throw "Generated Mod runtime schema check accepted semantic drift"
    }

    Write-Output (
        "MOD-RUNTIME-SCHEMA-CONSUMERS: PASS crlfAccepted=true semanticDriftRejected=true"
    )
} finally {
    $resolved = [IO.Path]::GetFullPath($tempRoot)
    $systemTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if ($resolved.StartsWith($systemTemp, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path -Leaf $resolved).StartsWith("nte-mod-runtime-schema-", [StringComparison]::Ordinal)) {
        Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction SilentlyContinue
    }
}
