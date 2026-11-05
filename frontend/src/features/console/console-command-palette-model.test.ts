import { describe, expect, it } from "vitest";

import japanese from "@res/languages/ja.json";
import simplifiedChinese from "@res/languages/zh-CN.json";

import { CONSOLE_PAGE_IDS } from "./console-navigation";
import {
  CONSOLE_COMMANDS,
  filterConsoleCommands,
  nextEnabledCommandIndex,
} from "./console-command-palette-model";

describe("Console command palette model", () => {
  it("keeps command IDs unique and exposes every Console page", () => {
    expect(new Set(CONSOLE_COMMANDS.map((command) => command.id)).size).toBe(
      CONSOLE_COMMANDS.length,
    );
    expect(
      CONSOLE_COMMANDS.filter((command) => command.action.kind === "navigate")
        .map((command) =>
          command.action.kind === "navigate" ? command.action.page : null,
        )
        .filter((page) => page !== null),
    ).toEqual(CONSOLE_PAGE_IDS);
  });

  it("matches translated, direct and subsequence queries", () => {
    const translate = (key: string) =>
      key === "Open Settings" ? "打开设置" : key;
    expect(
      filterConsoleCommands(CONSOLE_COMMANDS, "打开设置", translate)[0].id,
    ).toBe("console.settings");
    expect(
      filterConsoleCommands(CONSOLE_COMMANDS, "pcapng", translate),
    ).toHaveLength(2);
    expect(
      filterConsoleCommands(CONSOLE_COMMANDS, "opstg", translate).map(
        (command) => command.id,
      ),
    ).toContain("console.settings");
  });

  it("enables implemented controls and skips the remaining unavailable command", () => {
    expect(CONSOLE_COMMANDS[0].action.kind).toBe("control");
    const unavailable = CONSOLE_COMMANDS.findIndex(
      (command) => command.action.kind === "unavailable",
    );
    expect(unavailable).toBeGreaterThanOrEqual(0);
    const next = nextEnabledCommandIndex(CONSOLE_COMMANDS, unavailable, 1);
    expect(CONSOLE_COMMANDS[next].action.kind).not.toBe("unavailable");
    const previous = nextEnabledCommandIndex(CONSOLE_COMMANDS, next, -1);
    expect(CONSOLE_COMMANDS[previous].action.kind).not.toBe("unavailable");
  });

  it("keeps every palette label in the shared Chinese and Japanese dictionaries", () => {
    const keys = new Set([
      "Command palette",
      "Search commands",
      "No matching commands",
      "Disabled",
      ...CONSOLE_COMMANDS.flatMap((command) => [
        command.titleKey,
        command.categoryKey,
      ]),
    ]);
    for (const key of keys) {
      expect(simplifiedChinese).toHaveProperty(key);
      expect(japanese).toHaveProperty(key);
    }
  });
});
