import { getCurrentWindow } from "@tauri-apps/api/window";

import { invokeCommand } from "@/lib/tauri/command-error";

export const showMainDpsFromHud = () =>
  invokeCommand<void>("show_main_dps_from_hud");

export const startHudWindowDragging = () => getCurrentWindow().startDragging();
