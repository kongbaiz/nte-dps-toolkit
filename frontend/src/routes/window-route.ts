import { CONSOLE_WINDOW_LABEL } from "@/lib/tauri/mod-studio-contract";
import { HUD_WINDOW_LABEL } from "@/lib/tauri/technical-contract";

export type WindowRoute = "mod-studio" | "technical-hud" | "unsupported";

const WINDOW_ROUTES = new Map<string, WindowRoute>([
  [CONSOLE_WINDOW_LABEL, "mod-studio"],
  [HUD_WINDOW_LABEL, "technical-hud"],
]);

export function resolveWindowRoute(windowLabel: string): WindowRoute {
  return WINDOW_ROUTES.get(windowLabel) ?? "unsupported";
}
