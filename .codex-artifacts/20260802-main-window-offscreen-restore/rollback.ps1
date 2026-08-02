param([switch]$Apply)
$ErrorActionPreference='Stop'
$ArtifactRoot=Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot=Split-Path -Parent (Split-Path -Parent $ArtifactRoot)
$BaselineRoot=Join-Path $ArtifactRoot 'baseline'
$baselineManifest=Get-Content -Raw -LiteralPath (Join-Path $ArtifactRoot 'baseline-manifest.json') | ConvertFrom-Json
$modifiedManifest=Get-Content -Raw -LiteralPath (Join-Path $ArtifactRoot 'modified-manifest.json') | ConvertFrom-Json
foreach($entry in $baselineManifest){
  $path=Join-Path $BaselineRoot $entry.path
  $hash=(Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant()
  if($hash -ne $entry.sha256){ throw "Baseline hash mismatch: $($entry.path)" }
}
foreach($entry in $modifiedManifest){
  $path=Join-Path $RepoRoot $entry.path
  $hash=(Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant()
  if($hash -ne $entry.sha256){ throw "Current source changed; rollback stopped: $($entry.path)" }
}
foreach($entry in $baselineManifest){
  $source=Join-Path $BaselineRoot $entry.path
  $destination=Join-Path $RepoRoot $entry.path
  if($Apply){
    Copy-Item -LiteralPath $source -Destination $destination -Force
    $hash=(Get-FileHash -Algorithm SHA256 -LiteralPath $destination).Hash.ToLowerInvariant()
    if($hash -ne $entry.sha256){ throw "Restored hash mismatch: $($entry.path)" }
    Write-Output "RESTORED $($entry.path)"
  }else{
    Write-Output "WOULD_RESTORE $($entry.path)"
  }
}
if($Apply){Write-Output 'ROLLBACK_APPLIED_OK'}else{Write-Output 'ROLLBACK_DRY_RUN_OK'}
