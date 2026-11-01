import type { HudConfigSnapshot } from "@/lib/tauri/generated/HudConfigSnapshot";
import type { HudModule } from "@/lib/tauri/generated/HudModule";
import type { SettingsSnapshot } from "@/lib/tauri/generated/SettingsSnapshot";
import { compareDecimalStrings } from "@/lib/decimal-string";
import { type CommandError } from "@/lib/tauri/command-error";

export type SettingsPageState =
  | { status: "loading" }
  | { status: "ready"; snapshot: SettingsSnapshot }
  | { status: "error"; error: CommandError };

export interface HudModuleMove {
  dragged: HudModule;
  target: HudModule;
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
  return accepted === null || compareDecimalStrings(incoming, accepted) > 0;
}

export function shouldAcceptSettingsRefresh(
  accepted: string | null,
  incoming: string,
): boolean {
  return accepted === null || compareDecimalStrings(incoming, accepted) >= 0;
}

export function hudModuleVisible(
  hud: HudConfigSnapshot,
  module: HudModule,
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
  order: readonly HudModule[],
  module: HudModule,
  direction: "up" | "down",
): HudModuleMove | null {
  const index = order.indexOf(module);
  const targetIndex = direction === "up" ? index - 1 : index + 1;
  const target = order[targetIndex];
  if (target === undefined) {
    return null;
  }
  return {
    dragged: module,
    target,
    insertAfter: direction === "down",
  };
}
