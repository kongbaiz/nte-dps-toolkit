param(
    [Parameter(Mandatory = $true)]
    [string]$SnapshotRoot,
    [Parameter(Mandatory = $true)]
    [ValidateSet("Absent", "Enabled")]
    [string]$Expected
)

$ErrorActionPreference = "Stop"
$lib = Get-Content -Raw -LiteralPath (Join-Path $SnapshotRoot "src-tauri/src/lib.rs")
$main = Get-Content -Raw -LiteralPath (Join-Path $SnapshotRoot "src-tauri/src/windows/main_dps.rs")

$hasHook = $lib.Contains(".on_page_load(") -and $lib.Contains("webview.window().show()")
$hasHelper = $main.Contains("fn should_reveal_after_page_load") -and
             $main.Contains('label == MAIN_DPS_WINDOW_LABEL') -and
             $main.Contains("PageLoadEvent::Finished")
$hasScopeTest = $main.Contains("native_reveal_fallback_only_targets_finished_main_page") -and
                $main.Contains("CONSOLE_WINDOW_LABEL")

if ($Expected -eq "Absent") {
    if ($hasHook -or $hasHelper -or $hasScopeTest) {
        throw "Baseline unexpectedly contains the native page-load reveal fallback."
    }
    Write-Output "NATIVE_PAGE_LOAD_FALLBACK=ABSENT"
    Write-Output "BEHAVIOR=hidden main window still depends on the frontend ready handshake"
} else {
    if (-not ($hasHook -and $hasHelper -and $hasScopeTest)) {
        throw "Modified snapshot is missing part of the native page-load reveal fallback."
    }
    Write-Output "NATIVE_PAGE_LOAD_FALLBACK=ENABLED"
    Write-Output "SCOPE=main-dps:PageLoadEvent::Finished"
    Write-Output "OTHER_WINDOWS=not revealed by fallback"
}
