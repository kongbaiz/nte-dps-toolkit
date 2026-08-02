param([string]$WorkspaceRoot = "D:\NTE_DPS_TOOL")
$ErrorActionPreference = "Stop"
$manifest = Get-Content (Join-Path $PSScriptRoot "baseline-manifest.json") -Raw | ConvertFrom-Json
$source = Join-Path (Join-Path $PSScriptRoot "original") $manifest.path
$target = Join-Path $WorkspaceRoot $manifest.path
Copy-Item -LiteralPath $source -Destination $target -Force
$actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $target).Hash.ToLowerInvariant()
if ($actual -ne $manifest.sha256) { throw "Rollback hash mismatch: $($manifest.path)" }
Write-Output "ROLLBACK_RESTORED=1"
exit 0
