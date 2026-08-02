import { getCurrentWindow } from "@tauri-apps/api/window";

export interface DesktopWindowHandle {
  startDragging(): Promise<void>;
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  isAlwaysOnTop(): Promise<boolean>;
  setAlwaysOnTop(enabled: boolean): Promise<void>;
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
    isAlwaysOnTop: () => currentWindow().isAlwaysOnTop(),
    setAlwaysOnTop: (enabled: boolean) =>
      currentWindow().setAlwaysOnTop(enabled),
    close: () => currentWindow().close(),
  };
}

export const desktopWindowClient = createDesktopWindowClient();
