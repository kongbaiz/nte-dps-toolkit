import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";

interface DraggableWindow {
  startDragging(): Promise<void>;
}

export function showMainDpsFromHud(): Promise<void> {
  return invoke("show_main_dps_from_hud");
}

type CurrentWindowProvider = () => DraggableWindow;

export function startHudWindowDragging(
  currentWindow: CurrentWindowProvider = getCurrentWindow,
): Promise<void> {
  return currentWindow().startDragging();
}
