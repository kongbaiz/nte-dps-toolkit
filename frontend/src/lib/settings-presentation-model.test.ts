import { describe, expect, it } from "vitest";

import type { InterfaceSettingsSnapshot } from "@/lib/tauri/generated/InterfaceSettingsSnapshot";

import { settingsPresentationEqual } from "./settings-presentation-model";

const PRESENTATION: InterfaceSettingsSnapshot = {
  language: "zh-CN",
  darkMode: false,
  themePreset: "zinc",
  accent: "zinc",
  density: "cozy",
  reduceMotion: false,
  islandNotifications: true,
  islandOffsetX: 0,
};

describe("settings presentation model", () => {
  it("treats an unchanged Rust projection as the same presentation", () => {
    expect(settingsPresentationEqual(PRESENTATION, { ...PRESENTATION })).toBe(
      true,
    );
  });

  it("detects a presentation field change", () => {
    expect(
      settingsPresentationEqual(PRESENTATION, {
        ...PRESENTATION,
        accent: "blue",
      }),
    ).toBe(false);
  });
});
