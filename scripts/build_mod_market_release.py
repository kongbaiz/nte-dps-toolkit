"""Stage the seven compiled Release components; signing stays on the release host."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import struct

BASE_URL = "https://dps.o-na-ni.com/mods/v3/"
COMPONENTS = (
    ("nte-host", "host", "game/d3d12.dll", "Toolkit Host", "Toolkit 宿主", "Toolkit ホスト"),
    ("nte-loader", "loader", "tools/NTE-Loader.exe", "Toolkit Loader", "Toolkit 加载器", "Toolkit ローダー"),
    ("uetools-driver", "driver", "driver/uetools.sys", "Toolkit Driver", "Toolkit 驱动", "Toolkit ドライバー"),
    ("nte_plugincombat", "plugin", "game/plugins/NTE_PluginCombat.dll", "Combat Plugin", "战斗插件", "戦闘プラグイン"),
    ("nte_pluginuser", "plugin", "game/plugins/NTE_PluginUser.dll", "Account Plugin", "账号插件", "アカウントプラグイン"),
    ("nte_pluginnetwork", "plugin", "game/plugins/NTE_PluginNetwork.dll", "Network Plugin", "网络插件", "ネットワークプラグイン"),
    ("nte_pluginperformance", "plugin", "game/plugins/NTE_PluginPerformance.dll", "Performance Plugin", "性能插件", "パフォーマンスプラグイン"),
)
SUMMARIES = {
    "nte-host": {
        "en": "Provides the shared foundation for plugins and connects Toolkit to the game.",
        "zh-CN": "为各项插件提供运行支持，连接 Toolkit 与游戏。",
        "ja": "各プラグインの動作を支え、Toolkit とゲームをつなぐ共通の基盤です。",
    },
    "nte-loader": {
        "en": "Loads Toolkit components into the game so you can connect from the control panel.",
        "zh-CN": "将 Toolkit 组件加载到游戏中，供控制面板连接游戏并启用插件功能。",
        "ja": "Toolkit のコンポーネントをゲームへ読み込み、コントロールパネルからの接続とプラグインの利用を可能にします。",
    },
    "uetools-driver": {
        "en": "Provides system support for the Toolkit Loader to load components into the game.",
        "zh-CN": "为 Toolkit 加载器提供系统支持，配合加载器将组件载入游戏。",
        "ja": "Toolkit ローダーがコンポーネントをゲームへ読み込むためのシステム機能を提供します。",
    },
    "nte_plugincombat": {
        "en": "Records combat data for damage statistics, skill analysis, in-game displays and report exports.",
        "zh-CN": "记录战斗数据，提供伤害统计、技能分析、游戏内信息显示和战斗报告导出。",
        "ja": "戦闘データを記録し、ダメージ集計、スキル分析、ゲーム内の情報表示、レポート出力に対応します。",
    },
    "nte_pluginuser": {
        "en": "Reads character and inventory data, with previews and exports of account snapshots.",
        "zh-CN": "读取角色与背包数据，支持预览和导出账号快照。",
        "ja": "キャラクターと所持品のデータを読み取り、アカウントのスナップショットを確認・出力できます。",
    },
    "nte_pluginnetwork": {
        "en": "Records game network activity and exports diagnostic logs for troubleshooting connections.",
        "zh-CN": "记录游戏网络收发情况，导出诊断日志，帮助排查连接问题。",
        "ja": "ゲームのネットワーク送受信を記録し、接続の問題を調べるための診断ログを出力します。",
    },
    "nte_pluginperformance": {
        "en": "An extension for game performance features. Manage loading and disabling it in the control panel.",
        "zh-CN": "游戏性能相关的扩展组件，可在控制面板中管理加载与停用。",
        "ja": "ゲームのパフォーマンスに関連する拡張コンポーネントです。読み込みと無効化はコントロールパネルで管理できます。",
    },
}


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
        summary = SUMMARIES[identifier]
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
