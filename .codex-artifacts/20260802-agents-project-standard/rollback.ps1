param(
    [string]$Target = "D:\NTE_DPS_TOOL\AGENTS.md"
)
$ErrorActionPreference = "Stop"
$source = Join-Path $PSScriptRoot "AGENTS.before.md"
Copy-Item -LiteralPath $source -Destination $Target -Force
$expected = ((Get-Content -LiteralPath (Join-Path $PSScriptRoot "baseline.txt") | Where-Object { $_ -like "baseline_sha256=*" }) -split "=", 2)[1]
$actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Target).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "rollback hash mismatch: $actual" }
Write-Output "rollback_sha256=$actual"
