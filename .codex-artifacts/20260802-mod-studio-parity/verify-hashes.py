from pathlib import Path
import hashlib, json, sys
manifest = Path(sys.argv[1])
root = Path(sys.argv[2])
expected = json.loads(manifest.read_text(encoding='utf-8'))
errors = []
for rel, digest in expected.items():
    path = root / rel
    if not path.is_file():
        errors.append(f'missing:{rel}')
        continue
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    if actual != digest:
        errors.append(f'hash:{rel}:{actual}')
print(f'VERIFIED_FILES={len(expected)-len(errors)}')
print(f'VERIFY_ROOT={root.resolve()}')
if errors:
    print('\n'.join(errors))
    raise SystemExit(1)
