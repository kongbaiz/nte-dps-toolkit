# Combat detail parity verification

## Inputs

- Original snapshot: `original/` (14 files, SHA-256 in `original-hashes.json`)
- Modified snapshot: `modified-snapshot/` (14 files, SHA-256 in `modified-hashes.json`)
- Window capability set: `abyss-values`, `combat-details`, `console`, `main-dps`
- Detail routes: team and character variants of the `combat-details` window

## Baseline and modified behavior

Command:

```powershell
& '.\.codex-artifacts\20260802-combat-details-parity\verify-artifacts.ps1'
```

Literal output, exit status 0:

```text
BASELINE contractVersion=1 windowControlCapabilities=0/4 summary=false skillBreakdown=false targetHp=false
MODIFIED contractVersion=2 windowControlCapabilities=4/4 summary=true skillBreakdown=true targetHp=true
ARTIFACTS original=verified modified=verified workspace=matches-modified files=14
```

Interpretation:

- Baseline: only drag permission was present; the detail projection/page had no old-layout summary, skill composition, or target HP projection.
- Modified: minimize, toggle-maximize and close are explicitly granted to all four custom-titlebar windows; detail contract v2 projects the old-layout hierarchy for both team and character views.

## Rust/Tauri verification

Command:

```powershell
cargo fmt --check --manifest-path src-tauri\Cargo.toml
cargo check --manifest-path src-tauri\Cargo.toml
```

Literal result, exit statuses 0 / 0:

```text
Checking nte-dps-tool-tauri v0.3.6 (D:\NTE_DPS_TOOL\src-tauri)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.07s
```

Command:

```powershell
cargo test --manifest-path src-tauri\Cargo.toml
```

Literal result, exit status 0:

```text
test result: ok. 102 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Command (capability-aware application build, no installer bundle):

```powershell
.\frontend\node_modules\.bin\tauri.cmd build --debug --no-bundle
```

Literal result, exit status 0:

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 15.28s
Built application at: D:\NTE_DPS_TOOL\src-tauri\target\debug\nte-dps-tool-tauri.exe
```

Two earlier invocations from `frontend` (`pnpm tauri build --debug --no-bundle` and `pnpm --dir frontend exec tauri build --debug --no-bundle`) exited 1 because the CLI process cwd did not contain/discover `src-tauri/tauri.conf.json`; the corrected root-cwd command above passed.

## React verification

Command:

```powershell
pnpm typecheck
pnpm lint
pnpm test
pnpm build
```

Working directory: `frontend`

Literal results, exit statuses 0 / 0 / 0 / 0:

```text
Test Files  59 passed (59)
Tests  188 passed (188)
✓ 3126 modules transformed.
✓ built in 982ms
```

Targeted format command:

```powershell
pnpm exec prettier --check src/features/main-dps/main-dps-detail-page.tsx src/features/main-dps/main-dps-page.tsx src/lib/tauri/main-dps-client.ts src/lib/tauri/main-dps-detail-client.ts src/lib/tauri/main-dps-detail-contract.ts src/lib/tauri/main-dps-detail-contract.test.ts
```

Literal output, exit status 0:

```text
Checking formatting...
All matched files use Prettier code style!
```

Repository-wide `pnpm --dir frontend format:check` still exits 1 on 33 pre-existing files outside this task. Those files were left untouched to avoid an unrelated formatting sweep.

## Rollback verification

Command:

```powershell
& '.\.codex-artifacts\20260802-combat-details-parity\rollback.ps1' -CheckOnly
```

Literal output, exit status 0:

```text
Rollback check passed: 14 original files verified for D:\NTE_DPS_TOOL
```
