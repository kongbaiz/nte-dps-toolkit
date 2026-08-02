import { describe, expect, it, vi } from "vitest";

import { createDesktopWindowClient } from "./desktop-window-client";

describe("desktop window client", () => {
  it("routes every custom titlebar action through the current Tauri window", async () => {
    const handle = {
      startDragging: vi.fn(async () => undefined),
      minimize: vi.fn(async () => undefined),
      toggleMaximize: vi.fn(async () => undefined),
      isAlwaysOnTop: vi.fn(async () => false),
      setAlwaysOnTop: vi.fn(async () => undefined),
      close: vi.fn(async () => undefined),
    };
    const client = createDesktopWindowClient(() => handle);

    await client.startDragging();
    await client.minimize();
    await client.toggleMaximized();
    await expect(client.isAlwaysOnTop()).resolves.toBe(false);
    await client.setAlwaysOnTop(true);
    await client.close();

    expect(handle.startDragging).toHaveBeenCalledOnce();
    expect(handle.minimize).toHaveBeenCalledOnce();
    expect(handle.toggleMaximize).toHaveBeenCalledOnce();
    expect(handle.isAlwaysOnTop).toHaveBeenCalledOnce();
    expect(handle.setAlwaysOnTop).toHaveBeenCalledWith(true);
    expect(handle.close).toHaveBeenCalledOnce();
  });
});
