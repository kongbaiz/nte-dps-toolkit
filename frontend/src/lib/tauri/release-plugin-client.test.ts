import { describe, it, expect, vi } from "vitest";
import {
  RELEASE_ACTIONS,
  createReleasePluginClient,
  parseReleaseResult,
  requiresConfirmation,
  requiresIdle,
  parseHudOptions,
} from "./release-plugin-client";
describe("Release-only plugin controls", () => {
  it("validates native HUD readback and permits controls during capture", () => {
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
    expect(requiresIdle("hudConfigure")).toBe(false);
  });
  it("has no retired or developer command and guards destructive actions", () => {
    expect(new Set(RELEASE_ACTIONS.map((a) => a[1])).size).toBe(
      RELEASE_ACTIONS.length,
    );
    for (const id of [111, 112, 200])
      expect(RELEASE_ACTIONS.some((a) => Number(a[1]) === id)).toBe(false);
    expect(requiresConfirmation("shutdown")).toBe(true);
    expect(requiresIdle("combatReset")).toBe(true);
  });
  it("binds the native host identity and does not retry failures", async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce({
        command: 300,
        preview: "{}",
        totalBytes: 2,
        truncated: false,
      })
      .mockRejectedValueOnce({ code: "plugin_timeout" });
    const c = createReleasePluginClient({ invoke });
    await c.execute("userStatus", "123:456");
    expect(invoke).toHaveBeenNthCalledWith(1, "release_plugin_action", {
      action: "userStatus",
      expectedIdentity: "123:456",
      value: null,
      confirmed: false,
    });
    await expect(c.execute("userRefresh", "123:456")).rejects.toEqual({
      code: "plugin_timeout",
    });
    expect(invoke).toHaveBeenCalledTimes(2);
  });
  it("validates preview budgets and requires explicit truncation", () => {
    expect(
      parseReleaseResult(
        { command: 107, preview: "数", totalBytes: 3, truncated: false },
        107,
      ).preview,
    ).toBe("数");
    for (const r of [
      {},
      { command: 108, preview: "{}", totalBytes: 2, truncated: false },
      { command: 107, preview: "数", totalBytes: 2, truncated: false },
      { command: 107, preview: "{}", totalBytes: 100, truncated: false },
      {
        command: 107,
        preview: "a".repeat(8193),
        totalBytes: 8193,
        truncated: false,
      },
    ])
      expect(() => parseReleaseResult(r, 107)).toThrow();
  });
});

it("exposes the fixed prebattle action with confirmation and no automatic retry", async () => {
  expect(
    RELEASE_ACTIONS.find((action) => action[0] === "prebattleSkillStatus")?.[1],
  ).toBe(120);
  expect(
    RELEASE_ACTIONS.find((action) => action[0] === "prebattleSkillUnlock")?.[1],
  ).toBe(121);
  expect(requiresConfirmation("prebattleSkillUnlock")).toBe(true);
  expect(requiresConfirmation("prebattleSkillStatus")).toBe(false);
  const invoke = vi.fn().mockRejectedValue({ code: "plugin_timeout" });
  await expect(
    createReleasePluginClient({ invoke }).execute(
      "prebattleSkillUnlock",
      "123:456",
      null,
      true,
    ),
  ).rejects.toEqual({ code: "plugin_timeout" });
  expect(invoke).toHaveBeenCalledExactlyOnceWith("release_plugin_action", {
    action: "prebattleSkillUnlock",
    expectedIdentity: "123:456",
    value: null,
    confirmed: true,
  });
});
