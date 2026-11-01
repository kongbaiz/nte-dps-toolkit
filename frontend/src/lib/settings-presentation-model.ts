import type { InterfaceSettingsSnapshot } from "@/lib/tauri/generated/InterfaceSettingsSnapshot";

export function settingsPresentationEqual(
  left: InterfaceSettingsSnapshot,
  right: InterfaceSettingsSnapshot,
): boolean {
  return (
    left.language === right.language &&
    left.darkMode === right.darkMode &&
    left.themePreset === right.themePreset &&
    left.accent === right.accent &&
    left.density === right.density &&
    left.reduceMotion === right.reduceMotion &&
    left.islandNotifications === right.islandNotifications &&
    left.islandOffsetX === right.islandOffsetX
  );
}
