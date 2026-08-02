import {
  ABYSS_VALUES_WINDOW_LABEL,
  CONSOLE_WINDOW_LABEL,
  CHARACTER_DETAILS_WINDOW_LABEL,
  TEAM_DETAILS_WINDOW_LABEL,
  HUD_WINDOW_LABEL,
  MAIN_DPS_WINDOW_LABEL,
  NOTIFICATION_ISLAND_WINDOW_LABEL,
} from "@/lib/tauri/window-labels";

export type WindowRoute =
  | "abyss-values"
  | "combat-details"
  | "console"
  | "main-dps"
  | "notification-island"
  | "technical-hud"
  | "unsupported";

const WINDOW_ROUTES = new Map<string, WindowRoute>([
  [ABYSS_VALUES_WINDOW_LABEL, "abyss-values"],
  [CONSOLE_WINDOW_LABEL, "console"],
  [CHARACTER_DETAILS_WINDOW_LABEL, "combat-details"],
  [TEAM_DETAILS_WINDOW_LABEL, "combat-details"],
  [HUD_WINDOW_LABEL, "technical-hud"],
  [MAIN_DPS_WINDOW_LABEL, "main-dps"],
  [NOTIFICATION_ISLAND_WINDOW_LABEL, "notification-island"],
]);

export function resolveWindowRoute(windowLabel: string): WindowRoute {
  return WINDOW_ROUTES.get(windowLabel) ?? "unsupported";
}
