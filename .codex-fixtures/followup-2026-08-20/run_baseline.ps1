$ErrorActionPreference = 'Stop'

$repo = Join-Path $PSScriptRoot 'baseline-copy'
$log = Join-Path $PSScriptRoot 'BASELINE.log'
$summary = Join-Path $PSScriptRoot 'BASELINE_SUMMARY.txt'

Set-Content -LiteralPath $log -Value '' -Encoding utf8
Set-Content -LiteralPath $summary -Value '' -Encoding utf8

function Invoke-VerifiedCommand {
    param([string]$Label, [scriptblock]$Action)

    Add-Content -LiteralPath $log -Value "=== $Label ===" -Encoding utf8
    Write-Output "=== $Label ==="
    $started = Get-Date
    & $Action 2>&1 | ForEach-Object {
        Write-Output $_
        Add-Content -LiteralPath $log -Value $_ -Encoding utf8
    }
    $code = $LASTEXITCODE
    $elapsed = ((Get-Date) - $started).TotalSeconds
    $summaryLine = "$Label`tEXIT=$code`tSECONDS=$([math]::Round($elapsed, 2))"
    Write-Output $summaryLine
    Add-Content -LiteralPath $summary -Value $summaryLine -Encoding utf8
    if ($code -ne 0) {
        throw "$Label failed with exit $code"
    }
}

Push-Location $repo
try {
    Invoke-VerifiedCommand -Label 'cargo fmt --check' -Action { cargo fmt --check }
    Invoke-VerifiedCommand -Label 'cargo test --locked' -Action { cargo test --locked }
    Invoke-VerifiedCommand -Label 'cargo test CLI' -Action {
        cargo test --locked --no-default-features --features cli
    }
    Invoke-VerifiedCommand -Label 'cargo test Tauri' -Action {
        cargo test --locked --manifest-path src-tauri/Cargo.toml
    }
    Invoke-VerifiedCommand -Label 'verify architecture' -Action {
        pwsh -NoProfile -File scripts/verify_architecture.ps1
    }
}
finally {
    Pop-Location
}
