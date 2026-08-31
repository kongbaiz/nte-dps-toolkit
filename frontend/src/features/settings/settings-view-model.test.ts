import { describe, expect, it } from "vitest";

import type { HudConfigSnapshot } from "@/lib/tauri/technical-contract";

import {
  adjacentHudModuleMove,
  hudModuleVisible,
  settingsSectionPending,
  shouldAcceptSettingsGeneration,
  shouldAcceptSettingsRefresh,
} from "./settings-view-model";

function hudFixture(): HudConfigSnapshot {
  return {
    width: 380,
    moduleOrder: ["title", "summary", "status", "characters", "timeline"],
    showTitle: false,
    showTeamDps: true,
    showDuration: false,
    showTotalDamage: false,
    showCharacterRows: true,
    showDamageTaken: false,
    showAbyssHalf: false,
    showPassthroughState: false,
    showMiniTimeline: false,
  };
}

describe("Settings view model", () => {
  it("derives grouped module visibility without rewriting Rust rules", () => {
    const hud = hudFixture();

    expect(hudModuleVisible(hud, "summary")).toBe(true);
    expect(hudModuleVisible(hud, "status")).toBe(false);
    expect(hudModuleVisible(hud, "characters")).toBe(true);
  });

  it("builds insert-before and insert-after moves for adjacent arrows", () => {
    const order = hudFixture().moduleOrder;

    expect(adjacentHudModuleMove(order, "summary", "up")).toEqual({
      dragged: "summary",
      target: "title",
      insertAfter: false,
    });
    expect(adjacentHudModuleMove(order, "summary", "down")).toEqual({
      dragged: "summary",
      target: "status",
      insertAfter: true,
    });
    expect(adjacentHudModuleMove(order, "title", "up")).toBeNull();
    expect(adjacentHudModuleMove(order, "timeline", "down")).toBeNull();
  });

  it("blocks every settings section while a serialized mutation is pending", () => {
    expect(settingsSectionPending("update-preferences", "update")).toBe(true);
    expect(settingsSectionPending("update-preferences", "interface")).toBe(
      true,
    );
    expect(settingsSectionPending("module:timeline", "hud-modules")).toBe(true);
    expect(settingsSectionPending("module:timeline", "capture")).toBe(true);
    expect(settingsSectionPending(null, "capture")).toBe(false);
  });

  it("drops duplicate and stale settings generations", () => {
    expect(shouldAcceptSettingsGeneration(null, "5")).toBe(true);
    expect(shouldAcceptSettingsGeneration("5", "6")).toBe(true);
    expect(shouldAcceptSettingsGeneration("5", "5")).toBe(false);
    expect(shouldAcceptSettingsGeneration("5", "4")).toBe(false);
  });

  it("lets a manual refresh settle on the current generation", () => {
    expect(shouldAcceptSettingsRefresh(null, "5")).toBe(true);
    expect(shouldAcceptSettingsRefresh("5", "5")).toBe(true);
    expect(shouldAcceptSettingsRefresh("5", "6")).toBe(true);
    expect(shouldAcceptSettingsRefresh("5", "4")).toBe(false);
  });
});
