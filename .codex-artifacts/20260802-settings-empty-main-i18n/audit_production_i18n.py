from __future__ import annotations

import json
import re
from pathlib import Path


root = Path(__file__).resolve().parents[2]
frontend = root / "frontend" / "src"
literal = re.compile(r"\b(?:t|tf)\(\s*([\"'])(.*?)\1", re.DOTALL)
production_keys: set[str] = set()

for path in frontend.rglob("*"):
    if path.suffix not in {".ts", ".tsx"} or ".test." in path.name:
        continue
    source = path.read_text(encoding="utf-8")
    production_keys.update(match.group(2) for match in literal.finditer(source))

zh = json.loads((root / "res/languages/zh-CN.json").read_text(encoding="utf-8"))
ja = json.loads((root / "res/languages/ja.json").read_text(encoding="utf-8"))
missing_zh = sorted(production_keys - set(zh))
missing_ja = sorted(production_keys - set(ja))
locale_delta = sorted(set(zh) ^ set(ja))

print(f"production_keys={len(production_keys)}")
print(f"production_missing_zh={json.dumps(missing_zh, ensure_ascii=False)}")
print(f"production_missing_ja={json.dumps(missing_ja, ensure_ascii=False)}")
print(f"locale_key_delta={len(locale_delta)}")

if missing_zh or missing_ja or locale_delta:
    raise SystemExit(1)
