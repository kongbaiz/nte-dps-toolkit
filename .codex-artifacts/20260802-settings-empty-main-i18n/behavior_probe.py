from __future__ import annotations

import json
import sys
from pathlib import Path


root = Path(sys.argv[1]).resolve()


def read(relative: str) -> str:
    path = root / relative
    return path.read_text(encoding="utf-8") if path.is_file() else ""


settings_page = read("frontend/src/features/settings/settings-page.tsx")
settings_catalog = read("frontend/src/features/settings/settings-catalog.tsx")
settings_client = read("frontend/src/lib/tauri/settings-client.ts")
empty_page = read("frontend/src/features/empty-curtain/empty-curtain-page.tsx")
empty_grid = read("frontend/src/features/empty-curtain/equipment-canvas-grid.tsx")
main_page = read("frontend/src/features/main-dps/main-dps-page.tsx")
main_model = read("frontend/src/features/main-dps/main-dps-model.ts")
settings_rust = read("src-tauri/src/commands/settings.rs")
empty_rust = read("src-tauri/src/commands/empty_curtain.rs")
main_rust = read("src-tauri/src/contract/main_dps_detail.rs")
zh = json.loads(read("res/languages/zh-CN.json"))
ja = json.loads(read("res/languages/ja.json"))

required_i18n = {
    "Boss",
    "Failed to import team data.",
    "Team data file dialog failed.",
    "Toggle mouse passthrough while the combat HUD is active",
    "DPS: {}",
    "Share: {}%",
    "Taken: {}",
}

result = {
    "settings_browser_file_input": 'type="file"' in settings_catalog,
    "settings_native_import_command": "import_settings_team_data_file" in settings_client
    and "import_settings_team_data_file" in settings_rust,
    "settings_two_column_breakpoint_900": "min-[900px]:grid-cols-2" in settings_page,
    "settings_nic_empty_warning": "No usable NIC found; confirm Npcap is installed" in settings_catalog,
    "settings_standalone_title": '<h1 className="font-heading text-xl font-medium">{t("Settings")}</h1>' in settings_page,
    "empty_standalone_header": "Console Loadout" in empty_page,
    "empty_right_click_management": "onContextMenu" in empty_grid
    and "equipmentCanvasPointerIntent" in empty_grid,
    "empty_exact_instruction": "Right-click equipment to manage it through the plugin" in empty_page,
    "main_context_detail_action": "View combat details" in main_page,
    "main_unattributed_state": "mainCharacterListState" in main_page
    and "unattributed" in main_model,
    "main_capture_status_tone": "mainCaptureStatusTone" in main_page,
    "main_redetect_action": 't("Re-detect")' in main_page,
    "remaining_i18n_keys_present": required_i18n <= set(zh) and required_i18n <= set(ja),
    "scoped_tauri_clippy_patterns_fixed": "return tauri::async_runtime::spawn_blocking" not in empty_rust
    and "return match result" not in settings_rust
    and "&source\n" not in main_rust,
}
print(json.dumps(result, ensure_ascii=False, indent=2))
