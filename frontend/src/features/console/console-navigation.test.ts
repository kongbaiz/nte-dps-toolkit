import { describe, expect, it } from "vitest";

import { isConsolePageId } from "./console-navigation";

describe("Console navigation", () => {
  it("enables only migrated Console pages", () => {
    expect(isConsolePageId("settings")).toBe(true);
    expect(isConsolePageId("mod-studio")).toBe(true);
    expect(isConsolePageId("history")).toBe(false);
    expect(isConsolePageId("toString")).toBe(false);
  });
});
