# UI motion phase 5 verification

## Scope

- Native screen-level notification island wake/reassert and targeted refresh.
- Capture lifecycle, History, and Encrypted INI notification coverage.
- Replay event scheduling: authoritative events and `CaptureStopped` are consumed before optional full packet-debug payloads; the bounded debug lane drops overflow without changing semantic events.

## Input

- Replay sample: `logs\nte_raw_20260730_171321_106.pcapng`
- Size: `3,803,068` bytes

## Baseline observation

Command:

```powershell
$env:NTE_TEST_CAPTURE='D:\NTE_DPS_TOOL\logs\nte_raw_20260730_171321_106.pcapng'
cargo test stress_large_pcapng_import_with_bounded_event_lanes -- --ignored --nocapture
```

Literal result:

```text
large pcapng import completed: semantic_events=4800, debug_packets=3941, dropped_debug_packets=0
test result: ok. 1 passed; 0 failed
finished in 19.88s
exit status: 0
```

## Modified replay behavior

Command:

```powershell
$env:NTE_TEST_CAPTURE='D:\NTE_DPS_TOOL\logs\nte_raw_20260730_171321_106.pcapng'
cargo test --release stress_replay_finishes_before_optional_debug_drain -- --ignored --nocapture
```

Literal result:

```text
replay reached stopped in 2.5085993s: pending_events=0, dropped_debug_packets=1893
test result: ok. 1 passed; 0 failed
exit status: 0
```

Interpretation: semantic/reliable events remain lossless; only optional boxed `PacketDebug` payloads overflow. Release replay completion is again near the legacy two-second behavior on this sample.

## Focused validation

```text
cargo fmt --check --manifest-path src-tauri/Cargo.toml
exit status: 0

cargo check --manifest-path src-tauri/Cargo.toml
Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.89s
exit status: 0

cargo test core::live_capture::tests -- --nocapture
12 passed; 0 failed; 1 ignored
exit status: 0

cargo test --manifest-path src-tauri/Cargo.toml capture_lifecycle_notices_cover_replay_and_live_completion -- --nocapture
1 passed; 0 failed
exit status: 0

cargo test --manifest-path src-tauri/Cargo.toml windows::island::tests -- --nocapture
4 passed; 0 failed
exit status: 0

cargo test --manifest-path src-tauri/Cargo.toml commands::history::tests -- --nocapture
1 passed; 0 failed
exit status: 0

cargo test --manifest-path src-tauri/Cargo.toml commands::encrypted_ini::tests -- --nocapture
2 passed; 0 failed
exit status: 0
```

The Tauri test link step emitted the existing localized MSVC linker message about creating `.dll.lib` and `.dll.exp`; tests passed.

## Artifact verification

- `modified-files.zip`: reopened with Python `zipfile.testzip()`; all five member SHA-256 hashes matched the workspace files.
- `ui-motion-phase5.patch`: `git -c core.autocrlf=false apply --check` passed, then applied to an originals-only replay tree; all five SHA-256 hashes matched the workspace files.
- `rollback.ps1`: executed against a copy of all modified files; all five restored SHA-256 hashes matched `originals/`.
- This verification record was reopened after writing.
