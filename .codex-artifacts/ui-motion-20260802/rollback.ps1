param(
  [string]$TargetRoot = "D:\NTE_DPS_TOOL"
)

$ErrorActionPreference = "Stop"
$artifactRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$sourceRoot = Join-Path $artifactRoot "original"
$files = @(
  "frontend\src\index.css",
  "frontend\src\components\ui\button.tsx",
  "frontend\src\components\ui\switch.tsx",
  "frontend\src\features\console\console-page.tsx",
  "frontend\src\features\console\console-sidebar.tsx",
  "frontend\src\features\console\console-command-palette.tsx"
)

foreach ($file in $files) {
  $source = Join-Path $sourceRoot $file
  $destination = Join-Path $TargetRoot $file
  New-Item -ItemType Directory -Force -Path (Split-Path $destination) | Out-Null
  Copy-Item -LiteralPath $source -Destination $destination -Force
  $sourceHash = (Get-FileHash $source -Algorithm SHA256).Hash
  $destinationHash = (Get-FileHash $destination -Algorithm SHA256).Hash
  if ($sourceHash -ne $destinationHash) {
    throw "rollback hash mismatch: $file"
  }
}

Write-Output "rollback_restored=$($files.Count)"
Write-Output "target=$TargetRoot"
