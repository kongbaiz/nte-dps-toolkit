import { describe, expect, it, vi } from "vitest";

import { installBrowserContextMenuSuppression } from "./browser-context-menu";

describe("browser context menu suppression", () => {
  it("prevents the browser menu without stopping application handlers", () => {
    const target = new EventTarget();
    const applicationHandler = vi.fn();
    target.addEventListener("contextmenu", applicationHandler);
    const uninstall = installBrowserContextMenuSuppression(target);
    const event = new Event("contextmenu", {
      bubbles: true,
      cancelable: true,
    });

    expect(target.dispatchEvent(event)).toBe(false);
    expect(event.defaultPrevented).toBe(true);
    expect(applicationHandler).toHaveBeenCalledOnce();

    uninstall();
    const nextEvent = new Event("contextmenu", { cancelable: true });
    expect(target.dispatchEvent(nextEvent)).toBe(true);
    expect(nextEvent.defaultPrevented).toBe(false);
  });
});
