import type {
  CaptureSnapshot,
  HudConfigSnapshot,
  HudCharacterSnapshot,
  HudModuleId,
  HudSnapshot,
  TechnicalCommandError,
  TechnicalSnapshot,
} from "@/lib/tauri/technical-contract";
import { HUD_MODULE_IDS } from "@/lib/tauri/technical-contract";

export type TechnicalPageState =
  | { status: "loading" }
  | { status: "error"; error: TechnicalCommandError }
  | { status: "ready"; snapshot: TechnicalSnapshot };

export type BridgeTone = "ready" | "degraded" | "unknown";
export type HudDataTone = "empty" | "preview" | "live" | "unknown";
export type CaptureAction = "start" | "stop" | "pending";
export type HudSurfaceTone = "blurred" | "transparent";
export type HudModuleMoveDirection = "up" | "down";

export interface HudModuleMoveIntent {
  target: HudModuleId;
  insertAfter: boolean;
}

export function acceptSnapshot(
  current: TechnicalPageState,
  incoming: TechnicalSnapshot,
): TechnicalPageState {
  if (
    current.status === "ready" &&
    BigInt(incoming.sequence) <= BigInt(current.snapshot.sequence)
  ) {
    return current;
  }

  return { status: "ready", snapshot: incoming };
}

export function bridgeTone(status: string): BridgeTone {
  switch (status) {
    case "ready":
      return "ready";
    case "degraded":
      return "degraded";
    default:
      return "unknown";
  }
}

export function hudDataTone(state: string): HudDataTone {
  switch (state) {
    case "empty":
    case "preview":
    case "live":
      return state;
    default:
      return "unknown";
  }
}

export function captureAction(phase: string): CaptureAction {
  switch (phase) {
    case "idle":
    case "stopped":
    case "failed":
      return "start";
    case "running":
      return "stop";
    default:
      return "pending";
  }
}

export function captureMessage(capture: CaptureSnapshot): {
  key: string;
  arguments: string[];
} {
  return capture.issue === null
    ? {
        key: capture.messageKey,
        arguments: capture.messageArguments,
      }
    : {
        key: capture.issue.messageKey,
        arguments: capture.issue.messageArguments,
      };
}

export function hudSurfaceTone(passthrough: boolean): HudSurfaceTone {
  return passthrough ? "transparent" : "blurred";
}

export function hudCharacterName(character: HudCharacterSnapshot): string {
  return character.previewLabelSuffix === null
    ? character.name
    : `${character.previewLabelSuffix}`;
}

export function hudModulesInOrder(config: HudConfigSnapshot): HudModuleId[] {
  const knownModules = new Set<string>(HUD_MODULE_IDS);
  const seen = new Set<string>();
  return config.moduleOrder.filter((module): module is HudModuleId => {
    if (!knownModules.has(module) || seen.has(module)) {
      return false;
    }
    seen.add(module);
    return true;
  });
}

export function hudModuleConfiguredVisible(
  config: HudConfigSnapshot,
  module: HudModuleId,
): boolean {
  switch (module) {
    case "title":
      return config.showTitle;
    case "summary":
      return (
        config.showTeamDps ||
        config.showDuration ||
        config.showTotalDamage ||
        config.showDamageTaken
      );
    case "status":
      return config.showAbyssHalf || config.showPassthroughState;
    case "characters":
      return config.showCharacterRows;
    case "timeline":
      return config.showMiniTimeline;
  }
}

export function hudModuleLabelKey(module: HudModuleId): string {
  switch (module) {
    case "title":
      return "Title";
    case "summary":
      return "Summary";
    case "status":
      return "Status";
    case "characters":
      return "Character Ranking";
    case "timeline":
      return "Curve";
  }
}

export function hudModuleDropInsertAfter(
  pointerY: number,
  targetTop: number,
  targetHeight: number,
): boolean {
  return pointerY >= targetTop + targetHeight / 2;
}

export function hudModuleKeyboardMove(
  modules: HudModuleId[],
  module: HudModuleId,
  direction: HudModuleMoveDirection,
): HudModuleMoveIntent | null {
  const index = modules.indexOf(module);
  const targetIndex = direction === "up" ? index - 1 : index + 1;
  if (index < 0 || targetIndex < 0 || targetIndex >= modules.length) {
    return null;
  }

  return {
    target: modules[targetIndex],
    insertAfter: direction === "down",
  };
}

export function parseHudWidthDraft(value: string): number | null {
  if (value.trim() === "") {
    return null;
  }
  const width = Number(value);
  return Number.isSafeInteger(width) &&
    width >= -2_147_483_648 &&
    width <= 2_147_483_647
    ? width
    : null;
}

export function visibleHudModules(snapshot: HudSnapshot): HudModuleId[] {
  return hudModulesInOrder(snapshot.config).filter((module) => {
    switch (module) {
      case "title":
        return snapshot.config.showTitle;
      case "summary":
        return (
          snapshot.config.showTeamDps ||
          snapshot.config.showDuration ||
          snapshot.config.showTotalDamage ||
          snapshot.config.showDamageTaken
        );
      case "status":
        return (
          (snapshot.config.showAbyssHalf && snapshot.status.abyssDetected) ||
          snapshot.config.showPassthroughState
        );
      case "characters":
        return (
          snapshot.config.showCharacterRows && snapshot.characters.length > 0
        );
      case "timeline":
        return (
          snapshot.config.showMiniTimeline &&
          snapshot.timeline !== null &&
          snapshot.timeline.buckets.length > 0
        );
      default:
        return false;
    }
  });
}

export function formatHudNumber(value: number): string {
  return Math.round(value).toLocaleString("en-US");
}

export function formatHudDuration(value: number): string {
  return `${value.toFixed(1)}s`;
}
