import { describe, expect, it, vi } from "vitest";

import { startHudWindowDragging } from "./hud-window-client";

describe("HUD window client", () => {
  it("delegates explicit dragging to the current Tauri window", async () => {
    const startDragging = vi.fn().mockResolvedValue(undefined);

    await startHudWindowDragging(() => ({ startDragging }));

    expect(startDragging).toHaveBeenCalledOnce();
  });
});
