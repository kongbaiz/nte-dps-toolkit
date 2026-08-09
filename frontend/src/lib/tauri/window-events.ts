import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const MAIN_DPS_CONFIRMATION_REQUESTED_EVENT =
  "main-dps-confirmation-requested";
export const CONSOLE_NAVIGATE_EVENT = "console-navigate";

export function subscribeMainDpsConfirmationRequested(
  onRequested: (payload: "start" | "reset") => void,
): Promise<UnlistenFn> {
  return listen<string>(MAIN_DPS_CONFIRMATION_REQUESTED_EVENT, (event) => {
    if (event.payload === "start" || event.payload === "reset") {
      onRequested(event.payload);
    }
  });
}

export function subscribeConsoleNavigate(
  onNavigate: (payload: string) => void,
): Promise<UnlistenFn> {
  return listen<string>(CONSOLE_NAVIGATE_EVENT, (event) => {
    onNavigate(event.payload);
  });
}
