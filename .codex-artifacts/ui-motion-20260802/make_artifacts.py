from __future__ import annotations

import difflib
import hashlib
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile


ARTIFACT = Path(r"D:\NTE_DPS_TOOL\.codex-artifacts\ui-motion-20260802")
WORKSPACE = Path(r"D:\NTE_DPS_TOOL")
FILES = (
    "frontend/src/index.css",
    "frontend/src/components/ui/button.tsx",
    "frontend/src/components/ui/switch.tsx",
    "frontend/src/features/console/console-page.tsx",
    "frontend/src/features/console/console-sidebar.tsx",
    "frontend/src/features/console/console-command-palette.tsx",
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest().upper()


patch: list[str] = []
manifest: list[str] = []
with ZipFile(ARTIFACT / "modified-files.zip", "w", ZIP_DEFLATED) as archive:
    for relative in FILES:
        original = ARTIFACT / "original" / relative
        current = WORKSPACE / relative
        modified = ARTIFACT / "modified" / relative
        modified.parent.mkdir(parents=True, exist_ok=True)
        modified.write_bytes(current.read_bytes())
        archive.write(modified, relative)
        manifest.append(f"{sha256(modified)}  {relative}")
        patch.extend(
            difflib.unified_diff(
                original.read_text(encoding="utf-8").splitlines(keepends=True),
                modified.read_text(encoding="utf-8").splitlines(keepends=True),
                fromfile=f"a/{relative}",
                tofile=f"b/{relative}",
                n=3,
            )
        )

(ARTIFACT / "modified-sha256.txt").write_text(
    "\n".join(manifest) + "\n", encoding="utf-8"
)
(ARTIFACT / "ui-motion.patch").write_text(
    "".join(patch), encoding="utf-8", newline="\n"
)
print(f"modified_files={len(FILES)}")
print(f"patch_bytes={(ARTIFACT / 'ui-motion.patch').stat().st_size}")
print(f"zip_bytes={(ARTIFACT / 'modified-files.zip').stat().st_size}")
