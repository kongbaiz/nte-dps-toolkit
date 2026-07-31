import { describe, expect, it } from "vitest";

import type { HudConfigSnapshot } from "@/lib/tauri/technical-contract";

import {
  adjacentHudModuleMove,
  hudModuleVisible,
  hudOptionEnabled,
  settingsSectionPending,
  shouldAcceptSettingsGeneration,
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
  it("maps stable option identifiers onto the projected HUD config", () => {
    const hud = hudFixture();

    expect(hudOptionEnabled(hud, "team_dps")).toBe(true);
    expect(hudOptionEnabled(hud, "duration")).toBe(false);
    expect(hudOptionEnabled(hud, "character_rows")).toBe(true);
  });

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

  it("limits pending visuals to the section that owns the mutation", () => {
    expect(settingsSectionPending("update-preferences", "update")).toBe(true);
    expect(settingsSectionPending("update-preferences", "interface")).toBe(
      false,
    );
    expect(settingsSectionPending("module:timeline", "hud-modules")).toBe(true);
    expect(settingsSectionPending("module:timeline", "capture")).toBe(false);
  });

  it("drops duplicate and stale settings generations", () => {
    expect(shouldAcceptSettingsGeneration(null, "5")).toBe(true);
    expect(shouldAcceptSettingsGeneration("5", "6")).toBe(true);
    expect(shouldAcceptSettingsGeneration("5", "5")).toBe(false);
    expect(shouldAcceptSettingsGeneration("5", "4")).toBe(false);
  });
});
