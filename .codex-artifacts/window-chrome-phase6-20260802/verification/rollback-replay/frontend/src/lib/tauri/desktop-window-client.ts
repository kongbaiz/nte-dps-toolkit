import { getCurrentWindow } from "@tauri-apps/api/window";

export interface DesktopWindowHandle {
  startDragging(): Promise<void>;
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  close(): Promise<void>;
}

type WindowProvider = () => DesktopWindowHandle;

export function createDesktopWindowClient(
  currentWindow: WindowProvider = getCurrentWindow,
) {
  return {
    startDragging: () => currentWindow().startDragging(),
    minimize: () => currentWindow().minimize(),
    toggleMaximized: () => currentWindow().toggleMaximize(),
    close: () => currentWindow().close(),
  };
}

export const desktopWindowClient = createDesktopWindowClient();
