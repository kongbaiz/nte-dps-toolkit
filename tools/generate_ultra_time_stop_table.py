"""Generate ultra skill time-stop durations from exported GA and Montage JSON.

The input is the redacted CUE4Parse export directory produced by the local probe.
The generated table contains only game asset paths and timing metadata; it must
not include local client paths, AES keys, usmap paths, or raw captures.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


DEFAULT_EXPORTS_ROOT = Path("target/timestop-all-exports")
DEFAULT_OUTPUT = Path("res/data/skills/ultra_time_stop.json")


def load_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as file:
        return json.load(file)


def object_ref_name(reference: Any) -> str:
    if not isinstance(reference, dict):
        return ""
    value = reference.get("ObjectName")
    if not isinstance(value, str):
        return ""
    if "'" in value:
        parts = value.split("'")
        value = parts[-2] if len(parts) >= 2 and parts[-1] == "" else parts[-1]
    if ":" in value:
        value = value.rsplit(":", 1)[-1]
    return value


def game_asset_without_object(asset_path: str) -> str:
    if asset_path.startswith("/Game/"):
        asset_path = asset_path[len("/Game/") :]
    if "." in asset_path:
        asset_path = asset_path.rsplit(".", 1)[0]
    return asset_path.replace("\\", "/")


def report_targets(exports_root: Path) -> dict[str, str]:
    report_path = exports_root / "cue4parse_report.json"
    if not report_path.is_file():
        return {}
    report = load_json(report_path)
    result: dict[str, str] = {}
    for item in report.get("targets", []):
        output = item.get("output")
        target = item.get("target")
        if isinstance(output, str) and isinstance(target, str):
            result[target.lower().replace("\\", "/")] = output
        for match in item.get("matches", []):
            if not isinstance(match, str):
                continue
            normalized = match.replace("\\", "/")
            prefix = "HT/Content/"
            suffix = ".uasset"
            if normalized.lower().startswith(prefix.lower()) and normalized.lower().endswith(suffix):
                asset = normalized[len(prefix) : -len(suffix)]
                result[asset.lower()] = output
    return result


def resolve_export_path(
    exports_root: Path, target_index: dict[str, str], asset_path: str
) -> Path | None:
    asset = game_asset_without_object(asset_path)
    indexed = target_index.get(asset.lower())
    if indexed:
        path = exports_root / indexed
        if path.is_file():
            return path
    direct = exports_root / f"{asset}.json"
    if direct.is_file():
        return direct
    lower_asset = asset.lower()
    for path in exports_root.rglob("*.json"):
        relative = path.relative_to(exports_root).with_suffix("").as_posix().lower()
        if relative == lower_asset:
            return path
    return None


def char_id_from_ability_path(path: Path) -> int | None:
    for part in path.parts:
        if not part.startswith("Ability_"):
            continue
        parts = part.split("_", 2)
        if len(parts) >= 2 and parts[1].isdigit():
            return 1000 + int(parts[1])
    return None


def notify_rows(montage_path: Path) -> list[dict[str, Any]]:
    exports = load_json(montage_path)
    by_name = {
        item.get("Name"): item
        for item in exports
        if isinstance(item, dict) and isinstance(item.get("Name"), str)
    }
    rows: list[dict[str, Any]] = []
    for item in exports:
        if item.get("Type") != "AnimMontage":
            continue
        for notify in item.get("Properties", {}).get("Notifies", []):
            seconds = notify.get("LinkValue")
            if not isinstance(seconds, (int, float)):
                continue
            notify_object = None
            for key in ("Notify", "NotifyStateClass"):
                name = object_ref_name(notify.get(key))
                if name in by_name:
                    notify_object = by_name[name]
                    break
            if notify_object is None:
                continue
            tag = (
                notify_object.get("Properties", {})
                .get("TriggerEventTag", {})
                .get("TagName")
            )
            rows.append(
                {
                    "tag": tag,
                    "notify_type": notify_object.get("Type"),
                    "notify_name": notify_object.get("Name"),
                    "seconds": float(seconds),
                }
            )
    return rows


def ultra_montage_asset(default_object: dict[str, Any]) -> str | None:
    montages = default_object.get("Properties", {}).get("MontageToPlays", [])
    keyed_items = [(str(item.get("Key", "")), item) for item in montages]
    ordered = (
        [item for key, item in keyed_items if key == "Default"]
        + [item for key, item in keyed_items if "ultra" in key.lower()]
        + [item for key, item in keyed_items if "dissolve" not in key.lower()]
    )
    for item in ordered:
        asset_path = item.get("Value", {}).get("AssetPathName")
        if isinstance(asset_path, str) and asset_path:
            return asset_path
    return None


def trigger_end_tags(exports: list[dict[str, Any]]) -> list[str]:
    tags: list[str] = []
    for item in exports:
        if item.get("Type") != "HTGAComponent_AbilityGamePaused":
            continue
        for tag in item.get("Properties", {}).get("TriggerEndEventTags", []):
            if isinstance(tag, str) and tag not in tags:
                tags.append(tag)
    return tags


def select_end_ability_seconds(
    trigger_tags: list[str], notifies: list[dict[str, Any]]
) -> tuple[float | None, str | None, str]:
    for tag in trigger_tags:
        matches = [
            row
            for row in notifies
            if row.get("tag") == tag and isinstance(row.get("seconds"), float)
        ]
        if matches:
            seconds = min(row["seconds"] for row in matches)
            return seconds, tag, "ability_game_paused_trigger_end_tag"

    fallback = [
        row
        for row in notifies
        if row.get("notify_type") == "BP_TriggerEndAbilityEffect_C"
        and isinstance(row.get("seconds"), float)
    ]
    if fallback:
        seconds = min(row["seconds"] for row in fallback)
        return seconds, None, "trigger_end_ability_effect_notify"

    return None, None, "missing"


def generate(exports_root: Path) -> dict[str, Any]:
    target_index = report_targets(exports_root)
    rows: dict[str, Any] = {}
    for ga_path in sorted(
        (exports_root / "Blueprints" / "Abilities" / "Player").glob(
            "Ability_*/GA_*UltraSkill*.json"
        )
    ):
        exports = load_json(ga_path)
        default_object = next(
            (
                item
                for item in exports
                if isinstance(item.get("Name"), str)
                and item["Name"].startswith("Default__GA_")
            ),
            None,
        )
        char_id = char_id_from_ability_path(ga_path)
        if default_object is None or char_id is None:
            continue
        tags = trigger_end_tags(exports)
        if not tags:
            continue
        montage_asset = ultra_montage_asset(default_object)
        montage_path = (
            resolve_export_path(exports_root, target_index, montage_asset)
            if montage_asset
            else None
        )
        notifies = notify_rows(montage_path) if montage_path else []
        seconds, matched_tag, source = select_end_ability_seconds(tags, notifies)
        if seconds is None:
            continue
        rows[str(char_id)] = {
            "ability_id": ga_path.stem,
            "end_ability_event_seconds": round(seconds, 6),
            "trigger_end_event_tags": tags,
            "matched_trigger_end_event_tag": matched_tag,
            "source": source,
            "confidence": "high"
            if source == "ability_game_paused_trigger_end_tag"
            else "medium",
            "ga_asset": f"/Game/{ga_path.relative_to(exports_root).with_suffix('').as_posix()}",
            "montage_asset": montage_asset,
        }
    return {
        "version": 1,
        "source": "CUE4Parse GA/Montage JSON exports; local paths redacted",
        "characters": rows,
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate ultra skill time-stop duration table."
    )
    parser.add_argument("--exports-root", type=Path, default=DEFAULT_EXPORTS_ROOT)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()

    document = generate(args.exports_root)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"wrote {len(document['characters'])} rows to {args.output.as_posix()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
