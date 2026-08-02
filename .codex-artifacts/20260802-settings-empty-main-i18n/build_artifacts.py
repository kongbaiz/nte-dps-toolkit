from __future__ import annotations

import difflib
import hashlib
import json
import shutil
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
ARTIFACT = Path(__file__).resolve().parent
ORIGINAL = ARTIFACT / "original"
MODIFIED = ARTIFACT / "modified-snapshot"
FILES = [
    line.strip()
    for line in (ARTIFACT / "scoped-files.txt").read_text(encoding="utf-8-sig").splitlines()
    if line.strip()
]


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


if MODIFIED.exists():
    shutil.rmtree(MODIFIED)

modified_hashes: dict[str, str] = {}
changed: list[str] = []
patch_lines: list[str] = []

for rel in FILES:
    source = ROOT / rel
    destination = MODIFIED / rel
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    modified_hashes[rel] = sha256(destination)

    before = ORIGINAL / rel
    if before.is_file() and sha256(before) == modified_hashes[rel]:
        continue
    changed.append(rel)
    before_lines = (
        before.read_text(encoding="utf-8").splitlines(keepends=True)
        if before.is_file()
        else []
    )
    after_lines = destination.read_text(encoding="utf-8").splitlines(keepends=True)
    patch_lines.extend(
        difflib.unified_diff(
            before_lines,
            after_lines,
            fromfile=f"a/{rel}" if before.is_file() else "/dev/null",
            tofile=f"b/{rel}",
            lineterm="\n",
        )
    )

(ARTIFACT / "modified-hashes.json").write_text(
    json.dumps(modified_hashes, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)
(ARTIFACT / "changed-files.txt").write_text("\n".join(changed) + "\n", encoding="utf-8")
(ARTIFACT / "parity-fixes.patch").write_text("".join(patch_lines), encoding="utf-8")

print(f"snapshot_files={len(FILES)}")
print(f"changed_files={len(changed)}")
print(f"patch_bytes={(ARTIFACT / 'parity-fixes.patch').stat().st_size}")
