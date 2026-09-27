"""Stage the seven compiled Release components; signing stays on the release host."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import struct

BASE_URL = "https://dps.o-na-ni.com/mods/v3/"
COMPONENTS = (
    ("nte-host", "host", "game/d3d12.dll", "UE Tools Host", "UE Tools 宿主", "UE Tools ホスト"),
    ("nte-loader", "loader", "tools/NTE-Loader.exe", "UE Tools Loader", "UE Tools 加载器", "UE Tools ローダー"),
    ("uetools-driver", "driver", "driver/uetools.sys", "UE Tools Driver", "UE Tools 驱动", "UE Tools ドライバー"),
    ("nte_plugincombat", "plugin", "game/plugins/NTE_PluginCombat.dll", "Combat Plugin", "战斗插件", "戦闘プラグイン"),
    ("nte_pluginuser", "plugin", "game/plugins/NTE_PluginUser.dll", "Account Plugin", "账号插件", "アカウントプラグイン"),
    ("nte_pluginnetwork", "plugin", "game/plugins/NTE_PluginNetwork.dll", "Network Plugin", "网络插件", "ネットワークプラグイン"),
    ("nte_pluginperformance", "plugin", "game/plugins/NTE_PluginPerformance.dll", "Performance Plugin", "性能插件", "パフォーマンスプラグイン"),
)


def encoded(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def read_component(path, kind):
    if not path.is_file() or not 64 <= path.stat().st_size <= 64 * 1024 * 1024:
        raise ValueError("Missing or oversized Release component")
    data = path.read_bytes()
    pe = struct.unpack_from("<I", data, 60)[0]
    if data[:2] != b"MZ" or pe + 94 > len(data) or data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("Invalid PE component")
    machine, = struct.unpack_from("<H", data, pe + 4)
    flags, magic = struct.unpack_from("<HH", data, pe + 22)
    subsystem, = struct.unpack_from("<H", data, pe + 92)
    dll = bool(flags & 0x2000)
    if machine != 0x8664 or magic != 0x20b:
        raise ValueError("Component is not AMD64 PE32+")
    if kind in ("host", "plugin"):
        valid = dll and subsystem in (2, 3)
    elif kind == "loader":
        valid = not dll and subsystem in (2, 3)
    else:
        valid = not dll and subsystem == 1
    if not valid:
        raise ValueError("Component PE kind mismatch")
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source-commit", required=True)
    args = parser.parse_args()
    now = datetime.datetime.now(datetime.timezone.utc)
    # Read and validate the complete candidate set before creating the output.
    candidates = [(entry, read_component(args.release_dir / entry[2], entry[1])) for entry in COMPONENTS]
    args.output.mkdir(parents=True, exist_ok=False)
    packages = args.output / "v3/packages"
    packages.mkdir(parents=True)
    items = []
    for (identifier, kind, source, en, zh, ja), data in candidates:
        sha = hashlib.sha256(data).hexdigest()
        version = f"{now.year}.{now.month}.{now.day}+{sha[:12]}"
        filename = f"{identifier}-{version}{Path(source).suffix}"
        output = packages / filename
        output.write_bytes(data)
        if output.stat().st_size != len(data) or hashlib.sha256(output.read_bytes()).hexdigest() != sha:
            raise ValueError("Staged component readback failed")
        summary = {
            "en": f"Compiled Release {en}. Installation does not imply runtime or game compatibility.",
            "zh-CN": f"正式版{zh}组件。安装完成不代表已加载或通过实机兼容性验证。",
            "ja": f"Release 版の{ja}。インストールはロード完了やゲーム互換性の確認を意味しません。",
        }
        items.append({
            "id": identifier, "component": kind, "bindings": [f"component.{identifier}"],
            "localizations": {lang: {"name": name, "summary": summary[lang]} for lang, name in (("en", en), ("zh-CN", zh), ("ja", ja))},
            "version": version, "author": "NTE", "capabilities": [],
            "artifact": {"url": BASE_URL + "packages/" + filename, "size": len(data), "sha256": sha},
        })
    payload = encoded({"schema": 6, "published_at": now.strftime("%Y-%m-%dT%H:%M:%SZ"), "mods": items})
    (args.output / "payload.json").write_bytes(payload)
    release_id = hashlib.sha256(payload).hexdigest()
    manifest = {"releaseId": release_id, "sourceCommit": args.source_commit, "components": [
        {"file": "v3/packages/" + item["artifact"]["url"].rsplit("/", 1)[1], "size": item["artifact"]["size"], "sha256": item["artifact"]["sha256"]} for item in items
    ]}
    (args.output / "publication.json").write_bytes(encoded(manifest))
    print(json.dumps({"releaseId": release_id, "components": len(items), "totalBytes": sum(x["artifact"]["size"] for x in items)}))


if __name__ == "__main__":
    main()
