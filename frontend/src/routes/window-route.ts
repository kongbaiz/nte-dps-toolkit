import {
  ABYSS_VALUES_WINDOW_LABEL,
  CONSOLE_WINDOW_LABEL,
  HUD_WINDOW_LABEL,
} from "@/lib/tauri/window-labels";

export type WindowRoute =
  "abyss-values" | "console" | "technical-hud" | "unsupported";

const WINDOW_ROUTES = new Map<string, WindowRoute>([
  [ABYSS_VALUES_WINDOW_LABEL, "abyss-values"],
  [CONSOLE_WINDOW_LABEL, "console"],
  [HUD_WINDOW_LABEL, "technical-hud"],
]);

export function resolveWindowRoute(windowLabel: string): WindowRoute {
  return WINDOW_ROUTES.get(windowLabel) ?? "unsupported";
}
