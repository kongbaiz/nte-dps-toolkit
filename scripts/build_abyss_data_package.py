#!/usr/bin/env python3
"""Build the deterministic, content-addressed abyss data ZIP and manifest."""

from __future__ import annotations

import argparse
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo


TABLES = (
    "DT_MonsterStaticData_Abyss.json",
    "DT_MonsterPackData.json",
    "abyss_floor_monster_summary.json",
    "season_names_zh_cn.json",
)
DEFAULT_BASE_URL = "https://dps.o-na-ni.com/data/abyss/v1"


def minified_json(path: Path) -> bytes:
    document = json.loads(path.read_text(encoding="utf-8"))
    return json.dumps(
        document,
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")


def validate_sources(entries: dict[str, bytes]) -> None:
    static = json.loads(entries[TABLES[0]])
    pack = json.loads(entries[TABLES[1]])
    summary = json.loads(entries[TABLES[2]])
    seasons = json.loads(entries[TABLES[3]])
    if not isinstance(static, list) or not static or not isinstance(static[0].get("Rows"), dict):
        raise ValueError("monster static table must contain Rows")
    if not isinstance(pack, list) or not pack or not isinstance(pack[0].get("Rows"), dict):
        raise ValueError("monster pack table must contain Rows")
    if not isinstance(summary, dict) or not isinstance(summary.get("rows"), list) or not summary["rows"]:
        raise ValueError("floor summary must contain rows")
    if not isinstance(seasons, dict) or not seasons:
        raise ValueError("season names must be a non-empty object")


def build(source: Path, output: Path, base_url: str) -> dict[str, object]:
    entries = {name: minified_json(source / name) for name in TABLES}
    validate_sources(entries)

    content_hash = hashlib.sha256()
    for name in TABLES:
        content_hash.update(name.encode("utf-8"))
        content_hash.update(b"\0")
        content_hash.update(entries[name])
        content_hash.update(b"\0")
    data_version = content_hash.hexdigest()

    output.mkdir(parents=True, exist_ok=True)
    temporary = output / "abyss-data.tmp.zip"
    with ZipFile(temporary, "w", compression=ZIP_DEFLATED, compresslevel=9) as archive:
        for name in TABLES:
            info = ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            archive.writestr(info, entries[name], compress_type=ZIP_DEFLATED, compresslevel=9)

    archive_bytes = temporary.read_bytes()
    archive_sha256 = hashlib.sha256(archive_bytes).hexdigest()
    artifact_name = f"abyss-data-{archive_sha256}.zip"
    artifact = output / artifact_name
    temporary.replace(artifact)

    manifest = {
        "schema": 1,
        "dataVersion": data_version,
        "updatedAt": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "artifact": {
            "url": f"{base_url.rstrip('/')}/{artifact_name}",
            "sha256": archive_sha256,
            "size": len(archive_bytes),
            "uncompressedSize": sum(len(value) for value in entries.values()),
        },
    }
    manifest_path = output / "manifest.json"
    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, separators=(",", ":")) + "\n",
        encoding="utf-8",
        newline="\n",
    )

    for old_archive in output.glob("abyss-data-*.zip"):
        if old_archive != artifact:
            old_archive.unlink()

    result = {
        "manifest": str(manifest_path.resolve()),
        "artifact": str(artifact.resolve()),
        "dataVersion": data_version,
        "sha256": archive_sha256,
        "compressedBytes": len(archive_bytes),
        "uncompressedBytes": manifest["artifact"]["uncompressedSize"],
    }
    print(json.dumps(result, ensure_ascii=False))
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--source",
        type=Path,
        required=True,
        help="directory containing the four source tables",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/abyss-data-deploy"),
        help="output directory for manifest.json and the content-addressed ZIP",
    )
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL)
    args = parser.parse_args()
    build(args.source, args.output, args.base_url)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
