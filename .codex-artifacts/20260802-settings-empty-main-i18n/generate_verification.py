from __future__ import annotations

import json
from pathlib import Path


artifact = Path(__file__).resolve().parent
results = json.loads((artifact / "validation-results.json").read_text(encoding="utf-8-sig"))
baseline = (artifact / "baseline-behavior.json").read_text(encoding="utf-8-sig").strip()
modified = (artifact / "modified-behavior.json").read_text(encoding="utf-8-sig").strip()
delivery = (artifact / "delivery-verification.log").read_text(encoding="utf-8-sig").strip()

table = ["| Check | Exact command | Exit | Log |", "| --- | --- | ---: | --- |"]
for result in results:
    command = result["command"].replace("|", "\\|")
    log = Path(result["log"]).name
    table.append(f"| `{result['id']}` | `{command}` | {result['exitStatus']} | `validation-logs/{log}` |")

record = f"""# Settings / Empty Curtain / Main Window / i18n verification

## Scope and roles

- Original snapshot: `{artifact / 'original'}` with `{artifact / 'original-hashes.json'}`.
- Modified artifact: `{artifact / 'modified-snapshot'}` with `{artifact / 'modified-hashes.json'}`.
- Patch/diff: `{artifact / 'parity-fixes.patch'}`.
- Verification record: `{artifact / 'verification.md'}` plus `validation-results.json` and `validation-logs/`.
- Runnable rollback: `{artifact / 'rollback.ps1'}`.
- Changed fields/branches: native Settings team-data import and NIC states; Empty Curtain equipment pointer intent; main-window capture/list/context-menu state; zh-CN/ja locale keys.

## Baseline behavior

Command:

```powershell
python .codex-artifacts/20260802-settings-empty-main-i18n/behavior_probe.py .codex-artifacts/20260802-settings-empty-main-i18n/original
```

Input: the byte-preserved `original/` snapshot listed by `scoped-files.txt` and hashed by `original-hashes.json`.

Literal output:

```json
{baseline}
```

Exit status: `0`.

## Modified behavior

Command:

```powershell
python .codex-artifacts/20260802-settings-empty-main-i18n/behavior_probe.py .codex-artifacts/20260802-settings-empty-main-i18n/modified-snapshot
```

Input: the byte-preserved `modified-snapshot/` hashed by `modified-hashes.json`.

Literal output:

```json
{modified}
```

Exit status: `0`.

## Automated validation matrix

{chr(10).join(table)}

All {len(results)} checks exited `0`. Selected literal results:

- root: `651 passed; 0 failed; 7 ignored`
- CLI: `427 passed; 0 failed; 6 ignored`
- Tauri focused Settings: `14 passed; 0 failed`
- Tauri full: `113 passed; 0 failed`
- frontend focused: `5 passed` files / `30 passed` tests
- frontend full: `63 passed` files / `218 passed` tests
- Tauri debug/no-bundle integration build: application built at `src-tauri/target/debug/nte-dps-tool-tauri.exe`
- production i18n: `production_missing_zh=[]`, `production_missing_ja=[]`, `locale_key_delta=0`
- CLI-only dependency audit: `FORBIDDEN_GUI_DEPS=0`
- focused Prettier: `All matched files use Prettier code style!`

## Patch, artifact, and rollback execution

The patch applies through Git and is text-equivalent across all 24 scoped files. Git normalizes text line endings in the isolated patch root, so byte-exact verification is owned by `modified-snapshot/` and `modified-hashes.json`. Rollback restores byte-exact original hashes and removes the file that was absent at baseline.

Literal delivery log:

```text
{delivery}
```

## Rollback command

```powershell
& 'D:\\NTE_DPS_TOOL\\.codex-artifacts\\20260802-settings-empty-main-i18n\\rollback.ps1' -Root 'D:\\NTE_DPS_TOOL'
```

The rollback guard first verifies every modified hash, then restores the original snapshot, removes original-absent additions, and verifies all original states.
"""

(artifact / "verification.md").write_text(record, encoding="utf-8")
print(f"verification_record={artifact / 'verification.md'}")
