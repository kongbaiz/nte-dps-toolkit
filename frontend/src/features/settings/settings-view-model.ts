import {
  compareSettingsGeneration,
  isHudModuleId,
  type SettingsCommandError,
  type SettingsSnapshot,
} from "@/lib/tauri/settings-contract";
import type {
  HudConfigSnapshot,
  HudModuleId,
} from "@/lib/tauri/technical-contract";

export type SettingsPageState =
  | { status: "loading" }
  | { status: "ready"; snapshot: SettingsSnapshot }
  | { status: "error"; error: SettingsCommandError };

export interface HudModuleMove {
  dragged: HudModuleId;
  target: HudModuleId;
  insertAfter: boolean;
}

export type SettingsPendingSection =
  | "interface"
  | "update"
  | "capture"
  | "hotkeys"
  | "layout"
  | "main-dps-display"
  | "team-data"
  | "capture-files"
  | "abyss-values"
  | "hud-window"
  | "hud-modules"
  | "hud-editor";

export function settingsSectionPending(
  pendingAction: string | null,
  _section: SettingsPendingSection,
): boolean {
  return pendingAction !== null;
}

export function shouldAcceptSettingsGeneration(
  accepted: string | null,
  incoming: string,
): boolean {
  return accepted === null || compareSettingsGeneration(incoming, accepted) > 0;
}

export function shouldAcceptSettingsRefresh(
  accepted: string | null,
  incoming: string,
): boolean {
  return (
    accepted === null || compareSettingsGeneration(incoming, accepted) >= 0
  );
}

export function hudModuleVisible(
  hud: HudConfigSnapshot,
  module: HudModuleId,
): boolean {
  switch (module) {
    case "title":
      return hud.showTitle;
    case "summary":
      return (
        hud.showTeamDps ||
        hud.showDuration ||
        hud.showTotalDamage ||
        hud.showDamageTaken
      );
    case "status":
      return hud.showAbyssHalf || hud.showPassthroughState;
    case "characters":
      return hud.showCharacterRows;
    case "timeline":
      return hud.showMiniTimeline;
  }
}

export function adjacentHudModuleMove(
  order: readonly string[],
  module: HudModuleId,
  direction: "up" | "down",
): HudModuleMove | null {
  const index = order.indexOf(module);
  const targetIndex = direction === "up" ? index - 1 : index + 1;
  const target = order[targetIndex];
  if (index < 0 || target === undefined || !isHudModuleId(target)) {
    return null;
  }
  return {
    dragged: module,
    target,
    insertAfter: direction === "down",
  };
}
