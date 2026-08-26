[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repositoryRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repositoryRoot
try {
    $pattern = '\b(?:(?:pub(?:\(crate\))?|export)\s+)?const\s+(?<name>[A-Z0-9_]*(?:CONTRACT|PROTOCOL|SNAPSHOT|SCHEMA)_VERSION)(?:\s*:\s*u32)?\s*=\s*(?<version>[0-9]+)\s*;'

    function Get-VersionConstants {
        param(
            [Parameter(Mandatory)]
            [System.IO.FileInfo[]]$Files,
            [Parameter(Mandatory)]
            [string]$Kind
        )

        $constants = @{}
        foreach ($file in $Files) {
            $text = [IO.File]::ReadAllText($file.FullName)
            foreach ($match in [regex]::Matches($text, $pattern)) {
                $name = $match.Groups['name'].Value
                $version = [uint32]::Parse($match.Groups['version'].Value, [Globalization.CultureInfo]::InvariantCulture)
                if ($constants.ContainsKey($name)) {
                    throw "$Kind contract version constant is duplicated: $name"
                }
                $constants[$name] = $version
            }
        }
        return $constants
    }

    # Discover the full production trees. Intentional one-language protocols
    # are removed through the explicit manifest below; a new unpaired schema
    # therefore fails instead of being silently missed by a hand-picked list.
    $rustFiles = @(
        Get-ChildItem -LiteralPath 'src' -Filter '*.rs' -File -Recurse
        Get-ChildItem -LiteralPath 'src-tauri/src' -Filter '*.rs' -File -Recurse
    )
    $typescriptFiles = @(
        Get-ChildItem -LiteralPath 'frontend/src' -File -Recurse |
            Where-Object {
                $_.Extension -in @('.ts', '.tsx') -and
                $_.Name -notmatch '\.(?:test|spec)\.'
            }
    )
    $rust = Get-VersionConstants -Files $rustFiles -Kind 'Rust'
    $typescript = Get-VersionConstants -Files $typescriptFiles -Kind 'TypeScript'

    # The Rust domain name predates the Mod Studio adapter prefix. Normalize
    # this one documented alias before comparing the cross-language contract.
    if ($rust.ContainsKey('MOD_SDK_SCHEMA_VERSION')) {
        if ($rust.ContainsKey('MOD_STUDIO_SDK_SCHEMA_VERSION')) {
            throw 'Rust contract version alias collides: MOD_SDK_SCHEMA_VERSION.'
        }
        $rust['MOD_STUDIO_SDK_SCHEMA_VERSION'] = $rust['MOD_SDK_SCHEMA_VERSION']
        $rust.Remove('MOD_SDK_SCHEMA_VERSION')
    }

    $intentionalRustOnly = [ordered]@{
        # CLI JSON-RPC and file/update schemas have no TypeScript consumer.
        BATTLE_READ_CONTRACT_VERSION = 4
        MOD_MARKET_SCHEMA_VERSION = 4
        PROTOCOL_VERSION = 1
        UPDATER_PROTOCOL_VERSION = 1
        UPDATE_SCHEMA_VERSION = 1
    }
    foreach ($name in $intentionalRustOnly.Keys) {
        if (-not $rust.ContainsKey($name)) {
            throw "Intentional Rust-only version constant was not discovered: $name"
        }
        if ($typescript.ContainsKey($name)) {
            throw "Intentional Rust-only version constant now has a TypeScript peer; remove it from the manifest: $name"
        }
        if ($rust[$name] -ne $intentionalRustOnly[$name]) {
            throw "$name changed from the reviewed Rust-only version $($intentionalRustOnly[$name]) to $($rust[$name]); update the manifest deliberately."
        }
        $rust.Remove($name)
    }

    function Get-ParityErrors {
        param(
            [Parameter(Mandatory)] [hashtable]$Rust,
            [Parameter(Mandatory)] [hashtable]$TypeScript
        )
        $result = [Collections.Generic.List[string]]::new()
        foreach ($name in @($Rust.Keys + $TypeScript.Keys | Sort-Object -Unique)) {
            if (-not $Rust.ContainsKey($name)) {
                $result.Add("Rust is missing $name (TypeScript=$($TypeScript[$name])).")
            }
            elseif (-not $TypeScript.ContainsKey($name)) {
                $result.Add("TypeScript is missing $name (Rust=$($Rust[$name])).")
            }
            elseif ($Rust[$name] -ne $TypeScript[$name]) {
                $result.Add("$name differs (Rust=$($Rust[$name]), TypeScript=$($TypeScript[$name])).")
            }
        }
        return $result
    }

    # A synthetic drift case keeps the gate itself honest without editing the
    # working tree or depending on a particular product version number.
    $selfTestErrors = @(Get-ParityErrors -Rust @{ TEST_SCHEMA_VERSION = 1 } -TypeScript @{ TEST_SCHEMA_VERSION = 2 })
    if ($selfTestErrors.Count -ne 1 -or $selfTestErrors[0] -notmatch 'differs') {
        throw 'Contract version parity self-test did not reject semantic drift.'
    }
    $missingRustErrors = @(Get-ParityErrors -Rust @{} -TypeScript @{ TEST_SCHEMA_VERSION = 1 })
    $missingTypeScriptErrors = @(Get-ParityErrors -Rust @{ TEST_SCHEMA_VERSION = 1 } -TypeScript @{})
    if ($missingRustErrors.Count -ne 1 -or $missingRustErrors[0] -notmatch 'Rust is missing' -or
        $missingTypeScriptErrors.Count -ne 1 -or $missingTypeScriptErrors[0] -notmatch 'TypeScript is missing') {
        throw 'Contract version parity self-test did not reject a missing side.'
    }

    $duplicateRejected = $false
    $duplicateFixture = Join-Path ([IO.Path]::GetTempPath()) ("nte-contract-parity-{0}.rs" -f [Guid]::NewGuid().ToString('N'))
    try {
        [IO.File]::WriteAllText(
            $duplicateFixture,
            "pub const DUPLICATE_SCHEMA_VERSION: u32 = 1;`npub const DUPLICATE_SCHEMA_VERSION: u32 = 1;`n",
            [Text.UTF8Encoding]::new($false)
        )
        try {
            [void](Get-VersionConstants -Files @([IO.FileInfo]::new($duplicateFixture)) -Kind 'fixture')
        }
        catch {
            $duplicateRejected = $_.Exception.Message -match 'duplicated'
        }
    }
    finally {
        Remove-Item -LiteralPath $duplicateFixture -Force -ErrorAction SilentlyContinue
    }
    if (-not $duplicateRejected) {
        throw 'Contract version parity self-test accepted a duplicate constant.'
    }

    $names = @($rust.Keys + $typescript.Keys | Sort-Object -Unique)
    $errors = @(Get-ParityErrors -Rust $rust -TypeScript $typescript)

    if ($errors.Count -gt 0) {
        throw "Contract version parity failed:`n$($errors -join "`n")"
    }
    if ($names.Count -eq 0) {
        throw 'Contract version parity found no constants.'
    }

    $result = (
        "Contract version parity passed: {0} shared constants; rustOnly={1}; " +
        "semanticDriftRejected=true missingSideRejected=true duplicateRejected=true discoveryVerified=true."
    ) -f $names.Count, $intentionalRustOnly.Count
    Write-Output $result
}
finally {
    Pop-Location
}
