import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  tauriInvokeTransport,
  type InvokeTransport,
} from "@/lib/tauri/stream-client";

export interface DesktopWindowHandle {
  startDragging(): Promise<void>;
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  isAlwaysOnTop(): Promise<boolean>;
  close(): Promise<void>;
}

type WindowProvider = () => DesktopWindowHandle;

export function createDesktopWindowClient(
  currentWindow: WindowProvider = getCurrentWindow,
  transport: InvokeTransport = tauriInvokeTransport,
) {
  return {
    startDragging: () => currentWindow().startDragging(),
    minimize: () => currentWindow().minimize(),
    toggleMaximized: () => currentWindow().toggleMaximize(),
    isAlwaysOnTop: () => currentWindow().isAlwaysOnTop(),
    setAlwaysOnTop: async (enabled: boolean) => {
      await transport.invoke("set_desktop_window_always_on_top", { enabled });
    },
    close: () => currentWindow().close(),
  };
}

export const desktopWindowClient = createDesktopWindowClient();
