from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path


root = Path(sys.argv[1]).resolve()
manifest = Path(sys.argv[2]).resolve()
expected = json.loads(manifest.read_text(encoding="utf-8-sig"))
failures: list[str] = []
for rel, digest in expected.items():
    path = root / rel
    actual = hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else "missing"
    if actual != digest:
        failures.append(f"{rel}: expected={digest}, actual={actual}")

if failures:
    print("\n".join(failures))
    raise SystemExit(1)
print(f"verified_files={len(expected)}")
