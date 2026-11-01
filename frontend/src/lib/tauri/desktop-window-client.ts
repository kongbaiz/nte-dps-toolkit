import { getCurrentWindow } from "@tauri-apps/api/window";

import { invokeCommand } from "@/lib/tauri/command-error";

export const desktopWindowClient = {
  startDragging: () => getCurrentWindow().startDragging(),
  minimize: () => getCurrentWindow().minimize(),
  toggleMaximized: () => getCurrentWindow().toggleMaximize(),
  isAlwaysOnTop: () => getCurrentWindow().isAlwaysOnTop(),
  setAlwaysOnTop: (enabled: boolean) =>
    invokeCommand<void>("set_desktop_window_always_on_top", { enabled }),
  close: () => getCurrentWindow().close(),
};
