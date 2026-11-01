import { describe, expect, it } from "vitest";

import {
  parseHudOptions,
  requiresConfirmation,
  requiresIdle,
} from "./release-plugin-client";

describe("Release-only plugin controls", () => {
  it("parses native HUD readback and rejects malformed options", () => {
    const result = (preview: string) => ({
      command: 119,
      preview,
      totalBytes: preview.length,
      truncated: false,
    });
    expect(parseHudOptions(result('{"options":0}'))).toBe(0);
    expect(parseHudOptions(result('{"options":31}'))).toBe(31);
    for (const value of [
      "{}",
      '{"options":32}',
      '{"options":-1}',
      '{"options":1.5}',
      '{"options":"0"}',
    ])
      expect(() => parseHudOptions(result(value))).toThrow();
  });

  it("guards destructive actions", () => {
    expect(requiresIdle("hudConfigure")).toBe(false);
    expect(requiresIdle("combatReset")).toBe(true);
    expect(requiresConfirmation("shutdown")).toBe(true);
    expect(requiresConfirmation("prebattleSkillUnlock")).toBe(true);
    expect(requiresConfirmation("prebattleSkillStatus")).toBe(false);
  });
});
