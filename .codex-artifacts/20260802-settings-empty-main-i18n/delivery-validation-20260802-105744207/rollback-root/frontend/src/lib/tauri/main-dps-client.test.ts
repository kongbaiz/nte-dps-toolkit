import { describe, expect, it, vi } from "vitest";

import { startMainDpsWindowDragging } from "./main-dps-client";

describe("main DPS window client", () => {
  it("delegates title-bar dragging to the native window", async () => {
    const startDragging = vi.fn(async () => undefined);

    await startMainDpsWindowDragging(() => ({ startDragging }));

    expect(startDragging).toHaveBeenCalledOnce();
  });
});
