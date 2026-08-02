import type { InterfaceSettings } from "@/lib/tauri/settings-contract";

export function settingsPresentationEqual(
  left: InterfaceSettings,
  right: InterfaceSettings,
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
