# Startup window reveal verification

## Root cause
The configured main-dps WebView starts hidden. Its only reveal path waited for two frontend requestAnimationFrame callbacks. A hidden WebView2 can throttle those callbacks, so startup could reach a state where the process was alive but no interface was shown.

## Baseline behavior
Command:
    powershell -NoProfile -ExecutionPolicy Bypass -File "D:\NTE_DPS_TOOL\.codex-artifacts\20260802-startup-window-reveal\verify-behaviors.ps1" -SnapshotRoot "D:\NTE_DPS_TOOL\.codex-artifacts\20260802-startup-window-reveal\baseline" -Expected Absent
Literal output:
    NATIVE_PAGE_LOAD_FALLBACK=ABSENT
    BEHAVIOR=hidden main window still depends on the frontend ready handshake
Exit status: 0

## Modified behavior
Command:
    powershell -NoProfile -ExecutionPolicy Bypass -File "D:\NTE_DPS_TOOL\.codex-artifacts\20260802-startup-window-reveal\verify-behaviors.ps1" -SnapshotRoot "D:\NTE_DPS_TOOL\.codex-artifacts\20260802-startup-window-reveal\modified-snapshot" -Expected Enabled
Literal output:
    NATIVE_PAGE_LOAD_FALLBACK=ENABLED
    SCOPE=main-dps:PageLoadEvent::Finished
    OTHER_WINDOWS=not revealed by fallback
Exit status: 0

The existing frontend first-paint handshake remains present in frontend/src/lib/tauri/window-ready.ts; the new Rust page-load hook is a main-window-only fallback after PageLoadEvent::Finished.

## Automated validation
Commands:
    cargo fmt --check --manifest-path src-tauri/Cargo.toml
    cargo check --manifest-path src-tauri/Cargo.toml
    cargo test --manifest-path src-tauri/Cargo.toml
    git diff --check
Literal status record:
    tauri-fmt=0
    tauri-check=0
    tauri-test=0
    diff-check=0
The Tauri suite reported 98 passed, 0 failed.

## Rollback dry run
Command:
    powershell -NoProfile -ExecutionPolicy Bypass -File "D:\NTE_DPS_TOOL\.codex-artifacts\20260802-startup-window-reveal\rollback.ps1"
Literal output:
    WOULD_RESTORE src-tauri/src/lib.rs
    WOULD_RESTORE src-tauri/src/windows/main_dps.rs
    ROLLBACK_DRY_RUN_OK
Exit status: 0

Apply rollback:
    powershell -NoProfile -ExecutionPolicy Bypass -File "D:\NTE_DPS_TOOL\.codex-artifacts\20260802-startup-window-reveal\rollback.ps1" -Apply

## Patch replay verification
Command: git apply --check --directory=".codex-artifacts/20260802-startup-window-reveal/patch-check" changes.patch
Literal output: <empty>
Exit status: 0
Command: git apply --directory=".codex-artifacts/20260802-startup-window-reveal/patch-check" changes.patch
Literal output: <empty>
Exit status: 0
Result: normalized patched content equals modified snapshot for both files.
