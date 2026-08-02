# Parity follow-up verification record

Workspace input: `D:\NTE_DPS_TOOL`

## Baseline behavior

Command:

```powershell
& .\.codex-artifacts\20260802-parity-followup\verify-parity.ps1 `
  -Root .\.codex-artifacts\20260802-parity-followup\original `
  -Mode baseline
```

Literal output:

```text
abyss-separate-current-button=true expected=true
abyss-duplicate-team-roster=true expected=true
replay-atomic-content-gate=false expected=false
hud-direct-module-drag=false expected=false
hud-character-avatar=false expected=false
rank-independent-color-fallback=true expected=true
```

Exit status: `0`

## Modified behavior

Command:

```powershell
& .\.codex-artifacts\20260802-parity-followup\verify-parity.ps1 `
  -Root . `
  -Mode modified
```

Literal output:

```text
abyss-inline-team-avatars=true expected=true
abyss-current-team-availability=true expected=true
replay-atomic-content-gate=true expected=true
hud-direct-module-drag=true expected=true
hud-character-avatar=true expected=true
shared-avatar-color-projection=true expected=true
known-character-color-regression=true expected=true
hud-rank-independent-fallback=true expected=true
```

Exit status: `0`

## Patch application

Input: a fresh copy of `original/` at `patch-check/`.

Commands:

```powershell
git -C .\.codex-artifacts\20260802-parity-followup\patch-check apply --check ..\parity-followup.patch
git -C .\.codex-artifacts\20260802-parity-followup\patch-check apply ..\parity-followup.patch
& .\.codex-artifacts\20260802-parity-followup\verify-parity.ps1 `
  -Root .\.codex-artifacts\20260802-parity-followup\patch-check `
  -Mode modified
```

Literal output:

```text
patch-apply-check=PASS
abyss-inline-team-avatars=true expected=true
abyss-current-team-availability=true expected=true
replay-atomic-content-gate=true expected=true
hud-direct-module-drag=true expected=true
hud-character-avatar=true expected=true
shared-avatar-color-projection=true expected=true
known-character-color-regression=true expected=true
hud-rank-independent-fallback=true expected=true
patch-applied-behavior=PASS
```

Exit statuses: `0`, `0`, `0`

## Rollback precondition

Command:

```powershell
& .\.codex-artifacts\20260802-parity-followup\rollback.ps1 -CheckOnly
```

Literal output:

```text
modified-precondition=PASS
rollback-check-only=PASS
```

Exit status: `0`

## Automated validation

| Exact command | Literal result | Exit |
| --- | --- | ---: |
| `cargo test storage::resource::tests::character_avatar_color_matches_the_established_projection -- --exact` | `1 passed; 0 failed` | 0 |
| `cargo test core::hud::tests --lib` | `6 passed; 0 failed` | 0 |
| `cargo fmt --check` | no diagnostics | 0 |
| `cargo check` | `Finished dev profile` | 0 |
| `cargo test` | `655 passed; 0 failed; 7 ignored` | 0 |
| `cargo check --bin nte-dps-tool --features gui` | `Finished dev profile` | 0 |
| `cargo check --bin nte-core --no-default-features --features cli` | `Finished dev profile` | 0 |
| `cargo clippy --bin nte-dps-tool --features gui -- -D warnings` | `Finished dev profile` | 0 |
| `cargo clippy --bin nte-core --no-default-features --features cli -- -D warnings` | `Finished dev profile` | 0 |
| `cargo test --features gui` | `655 passed; 0 failed; 7 ignored` | 0 |
| `cargo test --no-default-features --features cli` | `429 passed; 0 failed; 6 ignored` | 0 |
| `cargo tree -e normal --no-default-features --features cli` plus banned dependency scan | `cli-banned-dependencies=NONE` | 0 |
| `cargo fmt --check --manifest-path src-tauri/Cargo.toml` | no diagnostics | 0 |
| `cargo check --manifest-path src-tauri/Cargo.toml` | `Finished dev profile` | 0 |
| `cargo test --manifest-path src-tauri/Cargo.toml` | `120 passed; 0 failed` | 0 |
| `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` | `Finished dev profile` | 0 |
| `pnpm --dir frontend lint` | `$ oxlint` | 0 |
| `pnpm --dir frontend typecheck` | `$ tsc -b` | 0 |
| `pnpm --dir frontend test` | `66 passed; 225 passed` | 0 |
| `pnpm --dir frontend build` | `3377 modules transformed; built` | 0 |
| task-file `prettier --check` | `All matched files use Prettier code style!` | 0 |
| `python -m json.tool res/languages/zh-CN.json` and `ja.json` | no diagnostics | 0 |
| task-scoped `git diff --check` | `json-and-task-diff-check=PASS` | 0 |
| `pnpm --dir frontend tauri:build -- --debug --no-bundle` | `Built application at: ...\src-tauri\target\debug\nte-dps-tool-tauri.exe` | 0 |

The final no-bundle build emitted the existing Windows linker informational warning and the Vite
large-chunk warning; the command completed successfully.

## Repository-wide format observation

Command: `pnpm --dir frontend format:check`

Literal result: `Code style issues found in 22 files.` All 22 reported paths are outside this task's
changed-file list; the task-file Prettier check above passed.

Exit status: `1`
