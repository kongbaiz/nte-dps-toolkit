# Mod Studio parity verification record

## Scope and inputs

- Baseline input: `original/` (18 pre-change files, SHA-256 in `original-hashes.json`).
- Modified input: `modified-snapshot/` (18 post-change files, SHA-256 in `modified-hashes.json`).
- Patch: `mod-studio-parity.patch`.
- Rollback: `rollback.ps1`.

## Exact baseline and modified behavior probe

Command:

```powershell
python .codex-artifacts\20260802-mod-studio-parity\behavior-probe.py .codex-artifacts\20260802-mod-studio-parity
```

Literal output; exit status 0:

```text
[baseline]
fixed_region_disabled=true
open_folder_action=false
create_document_action=false
manual_directory_action=false
china_global_options=false
loader_action=false
[modified]
fixed_region_disabled=false
open_folder_action=true
create_document_action=true
manual_directory_action=true
china_global_options=true
loader_action=true
behavior_probe_passed=true
```

## Modified artifact verification

Command:

```powershell
python .codex-artifacts\20260802-mod-studio-parity\verify-hashes.py .codex-artifacts\20260802-mod-studio-parity\modified-hashes.json D:\NTE_DPS_TOOL
```

Literal output; exit status 0:

```text
VERIFIED_FILES=18
VERIFY_ROOT=D:\NTE_DPS_TOOL
```

## Patch verification and execution

Commands:

```powershell
git apply --check --whitespace=error-all --directory=.codex-artifacts/20260802-mod-studio-parity/validation/patch-apply-root .codex-artifacts/20260802-mod-studio-parity/mod-studio-parity.patch
git apply --whitespace=error-all --directory=.codex-artifacts/20260802-mod-studio-parity/validation/patch-apply-root .codex-artifacts/20260802-mod-studio-parity/mod-studio-parity.patch
```

Literal output; both exit status 0:

```text
PATCH_CHECK_EXIT=0
PATCH_APPLY_EXIT=0
LOGICAL_VERIFIED_FILES=18
LOGICAL_VERIFY_EXIT=0
```

The isolated patch tree is `validation/patch-apply-root`. Logical line content is exact for all 18 files; the byte manifests remain the authority for the untouched original and exact modified snapshots because Git for Windows applies configured CRLF conversion.

## Rollback verification and execution

Command:

```powershell
& .codex-artifacts\20260802-mod-studio-parity\rollback.ps1 -WorkspaceRoot .codex-artifacts\20260802-mod-studio-parity\validation\rollback-root-verified
python .codex-artifacts\20260802-mod-studio-parity\verify-hashes.py .codex-artifacts\20260802-mod-studio-parity\original-hashes.json .codex-artifacts\20260802-mod-studio-parity\validation\rollback-root-verified
```

Literal output; exit status 0:

```text
ROLLBACK_RESTORED=18
VERIFIED_FILES=18
ROLLBACK_EXEC_SUCCESS=True
ROLLBACK_VERIFY_EXIT=0
```

## Focused verification

The literal command output is stored in `validation/focused-verification.log`.

| Command | Literal result | Exit |
| --- | --- | ---: |
| `cargo fmt --check` | no diagnostics | 0 |
| `cargo test --no-default-features --features desktop core::mod_studio::tests --lib` | `13 passed; 0 failed` | 0 |
| `cargo test --no-default-features --features desktop manual_game_directory_accepts_install_root_or_binary_directory --lib` | `1 passed; 0 failed` | 0 |
| `cargo test --manifest-path src-tauri/Cargo.toml mod_studio::tests` | `13 passed; 0 failed` | 0 |
| `pnpm --dir frontend exec vitest run src/lib/tauri/mod-studio-contract.test.ts src/lib/tauri/mod-studio-client.test.ts` | `2 passed (2)` files, `12 passed (12)` tests | 0 |
| task-file `prettier --check` | `All matched files use Prettier code style!` | 0 |
| `pnpm --dir frontend lint` | no diagnostics | 0 |
| `pnpm --dir frontend typecheck` | no diagnostics | 0 |
| `pnpm --dir frontend build` | `3371 modules transformed`, `built in 1.03s`; chunk-size advisory only | 0 |
| `git diff --check` | no whitespace errors | 0 |

## Full matrix already executed for this change

| Command | Literal result | Exit |
| --- | --- | ---: |
| `cargo check` | finished successfully | 0 |
| `cargo test` | `651 passed; 0 failed; 7 ignored` | 0 |
| `cargo check --bin nte-dps-tool --features gui` | finished successfully | 0 |
| `cargo test --features gui` | `651 passed; 0 failed; 7 ignored` | 0 |
| `cargo check --bin nte-core --no-default-features --features cli` | finished successfully | 0 |
| `cargo test --no-default-features --features cli` | `427 passed; 0 failed; 6 ignored` | 0 |
| `cargo clippy --bin nte-dps-tool --features gui -- -D warnings` | no diagnostics | 0 |
| `cargo clippy --bin nte-core --no-default-features --features cli -- -D warnings` | no diagnostics | 0 |
| `cargo tree -e normal --no-default-features --features cli` | forbidden GUI dependency count `0` | 0 |
| `cargo fmt --check --manifest-path src-tauri/Cargo.toml` | no diagnostics | 0 |
| `cargo check --manifest-path src-tauri/Cargo.toml` | finished successfully | 0 |
| `cargo test --manifest-path src-tauri/Cargo.toml` | `110 passed; 0 failed` | 0 |
| `pnpm --dir frontend test` | `62 passed (62)` files, `207 passed (207)` tests | 0 |

## Known unrelated repository-wide checks

- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` exits 101 on five existing findings in `empty_curtain.rs`, `settings.rs`, `history.rs`, and `main_dps_detail.rs`; no finding points to a Mod Studio task file. Literal output: `validation/tauri-clippy.log`.
- `pnpm --dir frontend format:check` exits 1 on 31 existing files outside this task's edited file set. All six edited TypeScript/TSX files pass the scoped Prettier check. Literal output: `validation/frontend-format-all.log`.

## UI execution boundary

The desktop UI was not launched. Project instructions reserve visual and native-window interaction checks for manual acceptance.
