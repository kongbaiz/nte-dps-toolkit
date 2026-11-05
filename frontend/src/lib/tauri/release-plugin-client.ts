import { invokeCommand } from "@/lib/tauri/command-error";
import type { ReleaseResult } from "@/lib/tauri/generated/ReleaseResult";

export type { ReleaseResult };

export const RELEASE_ACTIONS = [
  ["hostStatus", 2, "Host status"],
  ["shutdown", 11, "Unload host"],
  ["combatStatus", 100, "Collection status"],
  ["combatReset", 103, "Reset native combat report"],
  ["combatExport", 104, "Export native combat report"],
  ["combatStopExport", 105, "Stop and export native report"],
  ["combatOperation", 106, "Combat operation status"],
  ["combatReport", 107, "Preview native combat report"],
  ["evidenceStatus", 108, "Evidence recording status"],
  ["networkStatus", 109, "Plugin network status"],
  ["networkEnable", 110, "Plugin network recording"],
  ["networkFlush", 113, "Flush plugin network evidence"],
  ["runtimeRefresh", 114, "Refresh plugin runtime snapshot"],
  ["radarRefresh", 115, "Request plugin radar refresh"],
  ["traceEnable", 116, "Plugin change tracing"],
  ["traceClear", 117, "Clear plugin change trace"],
  ["hudStatus", 118, "In-game HUD status"],
  ["hudConfigure", 119, "In-game enhanced HUD"],
  ["prebattleSkillStatus", 120, "Prebattle skill unlock status"],
  ["prebattleSkillUnlock", 121, "Unlock prebattle skills"],
  ["frameRateStatus", 122, "Frame rate unlock status"],
  ["frameRateConfigure", 123, "Frame rate unlock"],
  ["frameRateRefresh", 124, "Refresh frame rate unlock"],
  ["quickUltimateStatus", 125, "Quick ultimate status"],
  ["quickUltimateConfigure", 126, "Quick ultimate"],
  ["trainingHideUid", 413, "Hide in-game UID"],
  ["trainingStatus", 400, "Training status"],
  ["trainingWoodenEnable", 403, "Enable wooden dummy wheel"],
  ["trainingWoodenDisable", 403, "Disable wooden dummy wheel"],
  ["trainingOperation", 410, "Training operation status"],
  ["userStatus", 300, "Account reader status"],
  ["userRefresh", 301, "Refresh account data"],
  ["userOperation", 302, "Account operation status"],
  ["userSnapshot", 303, "Preview account snapshot"],
  ["userExport", 304, "Export account snapshot"],
  ["userCancel", 305, "Cancel account refresh"],
] as const;
export type ReleaseAction = (typeof RELEASE_ACTIONS)[number][0];
export function parseHudOptions(result: ReleaseResult): number {
  if (![118, 119].includes(result.command) || result.truncated)
    throw new TypeError("invalid HUD result");
  const value: unknown = JSON.parse(result.preview);
  if (
    typeof value !== "object" ||
    value === null ||
    !("options" in value) ||
    typeof value.options !== "number" ||
    !Number.isInteger(value.options) ||
    value.options < 0 ||
    value.options > 31
  )
    throw new TypeError("invalid HUD options");
  return value.options;
}
export function requiresIdle(a: ReleaseAction) {
  return ["combatReset", "combatStopExport", "shutdown"].includes(a);
}
export function requiresConfirmation(a: ReleaseAction) {
  return [
    "combatReset",
    "combatStopExport",
    "shutdown",
    "traceClear",
    "userCancel",
    "prebattleSkillUnlock",
    "trainingWoodenEnable",
    "trainingWoodenDisable",
  ].includes(a);
}
export const releasePluginClient = {
  execute: (
    action: ReleaseAction,
    expectedIdentity: string,
    value: number | null = null,
    confirmed = false,
  ) =>
    invokeCommand<ReleaseResult>("release_plugin_action", {
      action,
      expectedIdentity,
      value,
      confirmed,
    }),
};
