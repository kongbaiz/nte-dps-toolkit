import { getCurrentWindow } from "@tauri-apps/api/window";

interface DraggableWindow {
  startDragging(): Promise<void>;
}

type CurrentWindowProvider = () => DraggableWindow;

export function startHudWindowDragging(
  currentWindow: CurrentWindowProvider = getCurrentWindow,
): Promise<void> {
  return currentWindow().startDragging();
}
