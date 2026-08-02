from __future__ import annotations

import argparse
import json
from pathlib import Path


ARTIFACT_ROOT = Path(__file__).resolve().parent
WORKSPACE_ROOT = ARTIFACT_ROOT.parent.parent


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def verify_baseline() -> None:
    originals = ARTIFACT_ROOT / "originals"
    titlebar = read(
        originals / "frontend/src/components/nte/desktop-titlebar.tsx"
    )
    main = read(originals / "frontend/src/features/main-dps/main-dps-page.tsx")
    entry = read(originals / "frontend/src/main.tsx")
    print(f"baseline_titlebar_pin={'<Pin' in titlebar}")
    print(f"baseline_toolbar_text_pin={'{t(\"Pin\")}' in main}")
    print(
        "baseline_context_menu_suppression="
        f"{'installBrowserContextMenuSuppression' in entry}"
    )


def verify_modified() -> None:
    root = WORKSPACE_ROOT
    configuration = json.loads(read(root / "src-tauri/tauri.conf.json"))
    windows = {window["label"]: window for window in configuration["app"]["windows"]}
    expected = {
        "main-dps",
        "hud-spike",
        "notification-island",
        "console",
        "abyss-values",
        "character-details",
        "team-details",
    }
    capability = json.loads(
        read(root / "src-tauri/capabilities/desktop-window-topmost.json")
    )
    titlebar = read(root / "frontend/src/components/nte/desktop-titlebar.tsx")
    main = read(root / "frontend/src/features/main-dps/main-dps-page.tsx")
    hud = read(root / "frontend/src/features/technical-hud/technical-hud-page.tsx")
    menu = read(root / "frontend/src/lib/browser-context-menu.ts")
    entry = read(root / "frontend/src/main.tsx")

    assert set(windows) == expected
    assert set(capability["windows"]) == expected - {"notification-island"}
    assert capability["permissions"] == ["core:window:allow-set-always-on-top"]
    assert "<Pin className=" in titlebar and "setAlwaysOnTop(enabled)" in titlebar
    assert "alwaysOnTop={snapshot.alwaysOnTop}" in main
    assert '{t("Pin")}' not in main
    assert "<Pin" in hud and "setAlwaysOnTop" in hud
    assert windows["notification-island"]["alwaysOnTop"] is True
    assert "event.preventDefault()" in menu and "capture: true" in menu
    assert "installBrowserContextMenuSuppression()" in entry

    print("configured_windows=" + ",".join(sorted(windows)))
    print(
        "titlebar_pin_windows="
        "abyss-values,character-details,console,main-dps,team-details"
    )
    print("hud_pin_control=verified")
    print("notification_island_always_on_top=true")
    print("native_context_menu_capture_suppression=verified")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("state", choices=("baseline", "modified"))
    args = parser.parse_args()
    if args.state == "baseline":
        verify_baseline()
    else:
        verify_modified()


if __name__ == "__main__":
    main()
