import { describe, expect, it } from "vitest";

import {
  consoleSidebarRowClasses,
  resolveConsoleSidebarPresentation,
} from "./console-sidebar-model";

describe("Console sidebar presentation", () => {
  it("keeps selected, hover, and disabled row colors mutually exclusive", () => {
    const idle = consoleSidebarRowClasses(false, false);
    const selected = consoleSidebarRowClasses(true, false);
    const disabled = consoleSidebarRowClasses(false, true);

    expect(idle).toContain("hover:bg-sidebar-accent");
    expect(selected).toContain("bg-sidebar-primary");
    expect(selected).not.toContain("hover:bg-sidebar-accent");
    expect(disabled).toContain("opacity-45");
    expect(disabled).not.toContain("hover:bg-sidebar-accent");
  });

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
