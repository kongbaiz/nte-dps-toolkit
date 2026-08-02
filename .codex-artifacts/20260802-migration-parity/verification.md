# Migration parity repair verification record

## Roles and inputs

- Original baseline: `original/` with `baseline.sha256` (43 captured pre-task files).
- Modified artifact: `modified-snapshot/` with `modified-snapshot.sha256` (44 changed files).
- Patch: `migration-parity.patch` (44 files, 2746 insertions, 251 deletions).
- Rollback: `rollback.ps1`; `-CheckOnly` verifies every input without editing the workspace.
- Complete literal transcripts: `verification-behavior.raw.log`, `verification-frontend-tauri.raw.log`, `verification-root-rust.raw.log`, `verification-cli-tree.raw.log`, `verification-tauri-final.raw.log`, `verification-diff.raw.log`, and `verification-artifacts.raw.log`.

## Baseline behavior

Command and input:

```powershell
$original = '.codex-artifacts\20260802-migration-parity\original'; @(
  "baseline_island_window=$([bool](Test-Path -LiteralPath (Join-Path $original 'src-tauri\src\windows\island.rs')))"
  "baseline_pause_snapshot=$([bool](Select-String -LiteralPath (Join-Path $original 'src-tauri\src\state.rs') -Pattern 'struct PausedPresentation' -Quiet))"
  "baseline_reset_undo=$([bool](Select-String -LiteralPath (Join-Path $original 'src-tauri\src\state.rs') -Pattern 'reset_session_with_undo' -Quiet))"
  "baseline_onboarding_flow=$([bool](Select-String -LiteralPath (Join-Path $original 'frontend\src\features\main-dps\main-dps-page.tsx') -Pattern 'onboardingStep' -Quiet))"
  "baseline_split_character_route=$([bool](Select-String -LiteralPath (Join-Path $original 'frontend\src\routes\window-route.ts') -Pattern 'character-details' -Quiet))"
  "baseline_abyss_team_import=$([bool](Select-String -LiteralPath (Join-Path $original 'frontend\src\features\abyss-values\abyss-values-page.tsx') -Pattern 'importCurrentTeam' -Quiet))"
)
```

Literal output:

```text
baseline_island_window=False
baseline_pause_snapshot=False
baseline_reset_undo=False
baseline_onboarding_flow=False
baseline_split_character_route=False
baseline_abyss_team_import=False
EXIT STATUS: 0
```

## Modified behavior

Focused Rust behavior commands and literal outputs:

```text
COMMAND: cargo test --manifest-path src-tauri/Cargo.toml pause_freezes_the_presented_state_until_resume
running 1 test
test state::tests::pause_freezes_the_presented_state_until_resume ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 119 filtered out
EXIT STATUS: 0

COMMAND: cargo test --manifest-path src-tauri/Cargo.toml reset_undo_restores_session_and_rejects_a_wrong_token_without_consuming_it
running 1 test
test state::tests::reset_undo_restores_session_and_rejects_a_wrong_token_without_consuming_it ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 119 filtered out
EXIT STATUS: 0

COMMAND: cargo test --manifest-path src-tauri/Cargo.toml onboarding_progress_and_completion_persist
running 1 test
test state::tests::onboarding_progress_and_completion_persist ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 119 filtered out
EXIT STATUS: 0

COMMAND: pnpm --dir frontend exec vitest run src/lib/tauri/island-contract.test.ts src/lib/tauri/main-dps-contract.test.ts src/routes/window-route.test.ts
Test Files  3 passed (3)
Tests  7 passed (7)
EXIT STATUS: 0
```

These checks verify the changed lifecycle branches (pause snapshot, reset/undo, onboarding persistence) and the new island/detail route contracts. Full-suite checks below cover the remaining profile, shortcut, geometry, team-import, and i18n integration.

## Final validation matrix

| Command | Literal result | Exit |
| --- | --- | ---: |
| `cargo fmt --check` | no output | 0 |
| `cargo check` | `Finished dev profile` | 0 |
| `cargo test` | `653 passed; 0 failed; 7 ignored` | 0 |
| `cargo check --bin nte-dps-tool --features gui` | `Finished dev profile` | 0 |
| `cargo check --bin nte-core --no-default-features --features cli` | `Finished dev profile` | 0 |
| `cargo clippy --bin nte-dps-tool --features gui -- -D warnings` | `Finished dev profile` | 0 |
| `cargo clippy --bin nte-core --no-default-features --features cli -- -D warnings` | `Finished dev profile` | 0 |
| `cargo test --features gui` | `653 passed; 0 failed; 7 ignored` | 0 |
| `cargo test --no-default-features --features cli` | `429 passed; 0 failed; 6 ignored` | 0 |
| `cargo tree -e normal --no-default-features --features cli` | tree written to `cli-dependency-tree.txt` | 0 |
| CLI banned-dependency scan | `PASS: CLI dependency tree contains none of tauri, wry, webview2-com, eframe, egui, wgpu, rfd, raw-window-handle` | 0 |
| `pnpm --dir frontend exec prettier --check <15 task files>` | `All matched files use Prettier code style!` | 0 |
| `pnpm --dir frontend lint` | `$ oxlint` | 0 |
| `pnpm --dir frontend typecheck` | `$ tsc -b` | 0 |
| `pnpm --dir frontend test` | `66 passed (66); 224 passed (224)` | 0 |
| `pnpm --dir frontend build` | `built in 1.03s` | 0 |
| `cargo fmt --check --manifest-path src-tauri/Cargo.toml` | no output | 0 |
| `cargo check --manifest-path src-tauri/Cargo.toml` | `Finished dev profile` | 0 |
| `cargo test --manifest-path src-tauri/Cargo.toml` | `120 passed; 0 failed` | 0 |
| `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` | `Finished dev profile` | 0 |
| `pnpm --dir frontend tauri:build -- --debug --no-bundle` | `Finished dev profile`; debug executable built | 0 |
| `git diff --check` | no errors; line-ending notices only | 0 |

`pnpm --dir frontend format:check` returned exit 1 because 29 pre-existing files outside this task fail repository-wide Prettier checking. The targeted task-file Prettier command above returned exit 0. The first Tauri formatting attempt also returned exit 1; formatting was corrected and the final Tauri formatting command returned exit 0. Both transcripts are retained verbatim.

## UI gate

No desktop UI was launched, clicked, or screenshot-tested. Manual sign-off is still required for 100%/125%/150% DPI, narrow windows, multi-window placement, notification focus behavior, drag/drop, shortcuts, passthrough, and Chinese/Japanese text.
