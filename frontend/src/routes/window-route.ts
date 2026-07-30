import { HUD_WINDOW_LABEL } from "@/lib/tauri/technical-contract";

export type WindowRoute = "technical-hud" | "unsupported";

const WINDOW_ROUTES = new Map<string, WindowRoute>([
  [HUD_WINDOW_LABEL, "technical-hud"],
]);

export function resolveWindowRoute(windowLabel: string): WindowRoute {
  return WINDOW_ROUTES.get(windowLabel) ?? "unsupported";
}
