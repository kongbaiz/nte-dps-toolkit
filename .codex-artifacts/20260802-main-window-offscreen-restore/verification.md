# Main window off-screen restore verification

## Runtime evidence
Input: debug executable adjacent config, selected geometry fields only.
Literal output:
{
  "source": "debug executable adjacent config",
  "mainWindowSize": [
    420.0,
    360.0
  ],
  "mainWindowPosition": [
    -16000.0,
    -16000.0
  ],
  "alwaysOnTop": true,
  "opacity": 1.0
}

## Baseline behavior
Command:
powershell -NoProfile -ExecutionPolicy Bypass -File verify-behaviors.ps1 -SnapshotRoot baseline -Expected Baseline
Literal output:
SENTINEL_RESTORE=ENABLED
MINIMIZED_GEOMETRY_PERSISTENCE=ENABLED
REVEAL_SEQUENCE=show-only
Exit status: 0

## Modified behavior
Command:
powershell -NoProfile -ExecutionPolicy Bypass -File verify-behaviors.ps1 -SnapshotRoot modified-snapshot -Expected Modified
Literal output:
SENTINEL_RESTORE=REJECTED_AND_CENTERED
MINIMIZED_GEOMETRY_PERSISTENCE=SKIPPED
REVEAL_SEQUENCE=show-unminimize-focus
Exit status: 0

## Automated validation
Commands:
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
git diff --check
Literal status:
tauri-fmt=0
tauri-check=0
tauri-test=0
diff-check=0
Test summary: 100 passed; 0 failed.

## Rollback dry run
Command:
powershell -NoProfile -ExecutionPolicy Bypass -File rollback.ps1
Literal output:
WOULD_RESTORE src-tauri/src/windows/main_dps.rs
WOULD_RESTORE src-tauri/src/lib.rs
ROLLBACK_DRY_RUN_OK
Exit status: 0
Apply rollback:
powershell -NoProfile -ExecutionPolicy Bypass -File rollback.ps1 -Apply

## Patch replay verification
Command: git apply --check --directory=.codex-artifacts/20260802-main-window-offscreen-restore/patch-check changes.patch
Literal output: <empty>
Exit status: 0
Command: git apply --directory=.codex-artifacts/20260802-main-window-offscreen-restore/patch-check changes.patch
Literal output: <empty>
Exit status: 0
Result: normalized patched content equals the modified snapshot for both files.
