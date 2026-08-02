param(
    [Parameter(Mandatory=$true)][string]$SnapshotRoot,
    [Parameter(Mandatory=$true)][ValidateSet("Baseline","Modified")][string]$Expected
)
$ErrorActionPreference='Stop'
$main=Get-Content -Raw -LiteralPath (Join-Path $SnapshotRoot 'src-tauri/src/windows/main_dps.rs')
$lib=Get-Content -Raw -LiteralPath (Join-Path $SnapshotRoot 'src-tauri/src/lib.rs')
$rejectsSentinel=$main.Contains('MINIMIZED_POSITION_LIMIT') -and
  $main.Contains('restorable_main_window_position') -and
  $main.Contains('minimized_windows_sentinel_is_not_restored')
$skipsTransient=$main.Contains('should_persist_geometry') -and
  $main.Contains('window.is_minimized()') -and
  $main.Contains('window.is_maximized()')
$frontendReveal=$main.Contains('window.unminimize()') -and $main.Contains('window.set_focus()')
$nativeReveal=$lib.Contains('window.unminimize()') -and $lib.Contains('window.set_focus()')
if($Expected -eq 'Baseline'){
  if($rejectsSentinel -or $skipsTransient -or $frontendReveal -or $nativeReveal){
    throw 'Baseline unexpectedly contains off-screen restore repair.'
  }
  Write-Output 'SENTINEL_RESTORE=ENABLED'
  Write-Output 'MINIMIZED_GEOMETRY_PERSISTENCE=ENABLED'
  Write-Output 'REVEAL_SEQUENCE=show-only'
}else{
  if(-not($rejectsSentinel -and $skipsTransient -and $frontendReveal -and $nativeReveal)){
    throw 'Modified snapshot is missing part of the off-screen restore repair.'
  }
  Write-Output 'SENTINEL_RESTORE=REJECTED_AND_CENTERED'
  Write-Output 'MINIMIZED_GEOMETRY_PERSISTENCE=SKIPPED'
  Write-Output 'REVEAL_SEQUENCE=show-unminimize-focus'
}
