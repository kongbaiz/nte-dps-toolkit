import { describe, expect, it } from "vitest";

import { resolveConsoleSidebarPresentation } from "./console-sidebar-model";

describe("Console sidebar presentation", () => {
  it("uses the effective collapsed state and hides the toggle at the breakpoint", () => {
    expect(resolveConsoleSidebarPresentation(false, false)).toEqual({
      collapsed: false,
      allowToggle: true,
    });
    expect(resolveConsoleSidebarPresentation(true, false)).toEqual({
      collapsed: true,
      allowToggle: true,
    });
    expect(resolveConsoleSidebarPresentation(false, true)).toEqual({
      collapsed: true,
      allowToggle: false,
    });
    expect(resolveConsoleSidebarPresentation(true, true)).toEqual({
      collapsed: true,
      allowToggle: false,
    });
  });
});
