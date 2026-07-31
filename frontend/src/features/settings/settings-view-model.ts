import {
  compareSettingsGeneration,
  isHudModuleId,
  type HudSettingOptionId,
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
  | "team-data"
  | "capture-files"
  | "abyss-values"
  | "hud-window"
  | "hud-modules"
  | "hud-editor";

export function settingsSectionPending(
  pendingAction: string | null,
  section: SettingsPendingSection,
): boolean {
  if (pendingAction === null) return false;
  switch (section) {
    case "interface":
      return pendingAction === "interface";
    case "update":
      return pendingAction.startsWith("update-");
    case "capture":
      return pendingAction === "capture" || pendingAction === "capture-devices";
    case "hotkeys":
      return (
        pendingAction === "hotkeys-enabled" ||
        pendingAction.startsWith("hotkey:")
      );
    case "layout":
      return pendingAction.startsWith("layout:");
    case "team-data":
      return pendingAction.startsWith("team-");
    case "capture-files":
      return pendingAction.startsWith("capture-files-");
    case "abyss-values":
      return pendingAction === "abyss-values";
    case "hud-window":
      return pendingAction === "always-on-top" || pendingAction === "width";
    case "hud-modules":
      return (
        pendingAction.startsWith("option:") ||
        pendingAction.startsWith("preset:") ||
        pendingAction.startsWith("module:") ||
        pendingAction.startsWith("move:")
      );
    case "hud-editor":
      return pendingAction === "open-editor";
  }
}

export function shouldAcceptSettingsGeneration(
  accepted: string | null,
  incoming: string,
): boolean {
  return accepted === null || compareSettingsGeneration(incoming, accepted) > 0;
}

export function hudOptionEnabled(
  hud: HudConfigSnapshot,
  option: HudSettingOptionId,
): boolean {
  switch (option) {
    case "title":
      return hud.showTitle;
    case "team_dps":
      return hud.showTeamDps;
    case "duration":
      return hud.showDuration;
    case "total_damage":
      return hud.showTotalDamage;
    case "damage_taken":
      return hud.showDamageTaken;
    case "character_rows":
      return hud.showCharacterRows;
    case "abyss_half":
      return hud.showAbyssHalf;
    case "passthrough_state":
      return hud.showPassthroughState;
    case "mini_timeline":
      return hud.showMiniTimeline;
  }
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
