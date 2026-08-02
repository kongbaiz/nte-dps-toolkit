# Main DPS parity verification

## Inputs

- Baseline snapshot: `baseline/`
- Modified snapshot: `modified-snapshot/`
- Baseline manifest: `baseline-manifest-expanded.json`
- Modified manifest: `modified-manifest.json`
- Repository branch: `codex/tauri-react-architecture`

## Baseline behavior command

```powershell
& '.codex-artifacts/20260802-main-dps-parity/verify-behaviors.ps1' -Snapshot Baseline
```

Literal output, exit status `0`:

```text
detail_actions_disabled=true
team_detail_client_absent=true
attribution_not_buttons=true
detail_route_absent=true
independent_monaco_theme=true
blue_native_dark_tint=true
```

## Modified behavior command

```powershell
& '.codex-artifacts/20260802-main-dps-parity/verify-behaviors.ps1' -Snapshot Modified
```

Literal output, exit status `0`:

```text
detail_actions_enabled_from_hits=true
team_detail_command_wired=true
attribution_buttons_wired=true
detail_route_present=true
auto_half_transition_test=true
compact_height_container_queries=true
monaco_follows_interface_theme=true
neutral_native_dark_tint=true
console_custom_titlebar=true
abyss_custom_titlebar=true
detail_custom_titlebar=true
```

## Validation matrix

| Command | Exit | Literal result |
| --- | ---: | --- |
| `cargo fmt --check` | 0 | no diff |
| `cargo check` | 0 | `Finished dev profile` |
| `cargo test` | 0 | `648 passed; 0 failed; 7 ignored` |
| `cargo check --bin nte-dps-tool --features gui` | 0 | `Finished dev profile` |
| `cargo check --bin nte-core --no-default-features --features cli` | 0 | `Finished dev profile` |
| `cargo clippy --bin nte-dps-tool --features gui -- -D warnings` | 0 | no warnings |
| `cargo clippy --bin nte-core --no-default-features --features cli -- -D warnings` | 0 | no warnings |
| `cargo test --features gui` | 0 | `648 passed; 0 failed; 7 ignored` |
| `cargo test --no-default-features --features cli` | 0 | `426 passed; 0 failed; 6 ignored` |
| `cargo tree -e normal --no-default-features --features cli` | 0 | forbidden GUI dependencies: `<none>` |
| `cargo fmt --check --manifest-path src-tauri/Cargo.toml` | 0 | no diff after rustfmt rerun |
| `cargo check --manifest-path src-tauri/Cargo.toml` | 0 | `Finished dev profile` |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 0 | `97 passed; 0 failed` |
| `pnpm --dir frontend lint` | 0 | no diagnostics |
| `pnpm --dir frontend typecheck` | 0 | no diagnostics |
| `pnpm --dir frontend test` | 0 | `59 passed (59)` and `187 passed (187)` |
| `pnpm --dir frontend build` | 0 | `3126 modules transformed` and `built in 1.03s` |
| targeted `prettier --check` for all task files | 0 | `All matched files use Prettier code style!` |
| `git diff --check` | 0 | line-ending warnings only; no whitespace error |
| `rollback.ps1` dry run | 0 | `ROLLBACK_DRY_RUN_OK` |

Full literal command output is retained under `validation/*.log`.

## Existing repository-wide checks

`pnpm --dir frontend format:check` exits `1` because 33 existing files outside this task do not match the
current Prettier configuration. Every frontend and documentation file changed by this task passes the targeted
Prettier check.

`cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` exits `101` on four existing diagnostics in
`commands/empty_curtain.rs`, `commands/settings.rs`, and `contract/history.rs`. The strict root GUI and CLI
Clippy commands pass, and the Tauri check/test matrix passes after formatting.

## Rollback command

Dry run:

```powershell
& '.codex-artifacts/20260802-main-dps-parity/rollback.ps1'
```

Apply:

```powershell
& '.codex-artifacts/20260802-main-dps-parity/rollback.ps1' -Apply
```
