import { describe, expect, it } from "vitest";

import {
  CONSOLE_PAGE_IDS,
  CONSOLE_FAVORITES_STORAGE_KEY,
  CONSOLE_SIDEBAR_STORAGE_KEY,
  DEFAULT_CONSOLE_PAGE,
  adjacentConsolePage,
  consolePageActivityMode,
  isConsolePageId,
  readConsoleSidebarCollapsed,
  readConsoleFavoritePages,
  resolveConsoleShortcut,
  writeConsoleSidebarCollapsed,
  writeConsoleFavoritePages,
} from "./console-navigation";

describe("Console navigation", () => {
  it("enables only migrated Console pages", () => {
    expect(isConsolePageId("settings")).toBe(true);
    expect(isConsolePageId("shortcuts")).toBe(true);
    expect(isConsolePageId("mod-studio")).toBe(true);
    expect(isConsolePageId("history")).toBe(true);
    expect(isConsolePageId("timeline")).toBe(true);
    expect(isConsolePageId("skills")).toBe(true);
    expect(isConsolePageId("empty-curtain")).toBe(true);
    expect(isConsolePageId("character-data")).toBe(true);
    expect(isConsolePageId("encrypted-ini")).toBe(true);
    expect(isConsolePageId("packets")).toBe(true);
    expect(isConsolePageId("resources")).toBe(false);
    expect(isConsolePageId("diagnostics")).toBe(true);
    expect(isConsolePageId("toString")).toBe(false);
  });

  it("keeps exactly the active page visible", () => {
    for (const activePage of CONSOLE_PAGE_IDS) {
      expect(
        CONSOLE_PAGE_IDS.map((page) =>
          consolePageActivityMode(activePage, page),
        ).filter((mode) => mode === "visible"),
      ).toHaveLength(1);
      for (const page of CONSOLE_PAGE_IDS) {
        expect(consolePageActivityMode(activePage, page)).toBe(
          page === activePage ? "visible" : "hidden",
        );
      }
    }
  });

  it("opens Settings and cycles through the sidebar order", () => {
    expect(DEFAULT_CONSOLE_PAGE).toBe("settings");
    expect(adjacentConsolePage("settings", -1)).toBe("diagnostics");
    expect(adjacentConsolePage("settings", 1)).toBe("shortcuts");
    expect(adjacentConsolePage("shortcuts", 1)).toBe("history");
    expect(adjacentConsolePage("empty-curtain", 1)).toBe("mod-studio");
    expect(adjacentConsolePage("mod-studio", 1)).toBe("character-data");
  });

  it("persists a validated and user-editable favorites list", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    expect(readConsoleFavoritePages(storage)).toEqual(["settings", "history"]);
    writeConsoleFavoritePages(["shortcuts"], storage);
    expect(values.get(CONSOLE_FAVORITES_STORAGE_KEY)).toBe('["shortcuts"]');
    expect(readConsoleFavoritePages(storage)).toEqual(["shortcuts"]);

    values.set(
      CONSOLE_FAVORITES_STORAGE_KEY,
      JSON.stringify(["history", "missing", "history", "packets"]),
    );
    expect(readConsoleFavoritePages(storage)).toEqual(["history", "packets"]);
    values.set(CONSOLE_FAVORITES_STORAGE_KEY, "not-json");
    expect(readConsoleFavoritePages(storage)).toEqual(["settings", "history"]);
  });

  it("persists the explicit sidebar preference without requiring WebView storage", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    expect(readConsoleSidebarCollapsed(storage)).toBe(false);
    writeConsoleSidebarCollapsed(true, storage);
    expect(values.get(CONSOLE_SIDEBAR_STORAGE_KEY)).toBe("true");
    expect(readConsoleSidebarCollapsed(storage)).toBe(true);
    writeConsoleSidebarCollapsed(false, storage);
    expect(readConsoleSidebarCollapsed(storage)).toBe(false);
    expect(readConsoleSidebarCollapsed(null)).toBe(false);
  });

  it("maps the legacy Console keyboard shortcuts and protects form controls", () => {
    const shortcut = (key: string, editable = false) =>
      resolveConsoleShortcut({
        key,
        ctrlKey: true,
        altKey: false,
        shiftKey: false,
        repeat: false,
        editable,
      });

    expect(shortcut("PageUp")).toBe("previous-page");
    expect(shortcut("PageDown")).toBe("next-page");
    expect(shortcut("PageUp", true)).toBeNull();
    expect(shortcut("k", true)).toBe("command-palette");
    expect(
      resolveConsoleShortcut({
        key: "PageDown",
        ctrlKey: true,
        altKey: false,
        shiftKey: true,
        repeat: false,
        editable: false,
      }),
    ).toBeNull();
  });
});
