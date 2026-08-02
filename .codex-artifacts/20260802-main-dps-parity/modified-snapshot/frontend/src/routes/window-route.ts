import {
  ABYSS_VALUES_WINDOW_LABEL,
  CONSOLE_WINDOW_LABEL,
  COMBAT_DETAILS_WINDOW_LABEL,
  HUD_WINDOW_LABEL,
  MAIN_DPS_WINDOW_LABEL,
} from "@/lib/tauri/window-labels";

export type WindowRoute =
  | "abyss-values"
  | "combat-details"
  | "console"
  | "main-dps"
  | "technical-hud"
  | "unsupported";

const WINDOW_ROUTES = new Map<string, WindowRoute>([
  [ABYSS_VALUES_WINDOW_LABEL, "abyss-values"],
  [CONSOLE_WINDOW_LABEL, "console"],
  [COMBAT_DETAILS_WINDOW_LABEL, "combat-details"],
  [HUD_WINDOW_LABEL, "technical-hud"],
  [MAIN_DPS_WINDOW_LABEL, "main-dps"],
]);

export function resolveWindowRoute(windowLabel: string): WindowRoute {
  return WINDOW_ROUTES.get(windowLabel) ?? "unsupported";
}
