from __future__ import annotations

import sys
from pathlib import Path


actual_root = Path(sys.argv[1]).resolve()
expected_root = Path(sys.argv[2]).resolve()
file_list = Path(sys.argv[3]).resolve()
files = [
    line.strip()
    for line in file_list.read_text(encoding="utf-8-sig").splitlines()
    if line.strip()
]

failures: list[str] = []
for relative in files:
    actual = actual_root / relative
    expected = expected_root / relative
    if not actual.is_file() or not expected.is_file():
        failures.append(f"missing file: {relative}")
        continue
    if actual.read_text(encoding="utf-8").splitlines() != expected.read_text(
        encoding="utf-8"
    ).splitlines():
        failures.append(f"content mismatch: {relative}")

if failures:
    print("\n".join(failures))
    raise SystemExit(1)
print(f"text_equivalent_files={len(files)}")
