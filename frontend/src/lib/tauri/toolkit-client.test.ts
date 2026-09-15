import { describe, expect, it, vi } from "vitest";
import { createToolkitClient, parsePluginPanel } from "./toolkit-client";

const capture = {
  contractVersion: 1,
  mode: "packet_capture",
  connection: "notRequested",
  directorySelected: false,
  plugins: [],
  combat: null,
  operation: null,
};
const connected = {
  ...capture,
  mode: "plugin",
  connection: "connected",
  plugins: [{ file: "NTE_PluginCombat.dll", state: "loaded" }],
  combat: {
    generation: "18446744073709551615",
    capturing: true,
    hits: "9007199254740993",
    totalDamage: 123.456789,
    direct: 120,
    correlated: 0,
    inferred: 3.456789,
    unknown: 0,
  },
  operation: { state: "", error: "" },
};

describe("plugin panel contract and routing", () => {
  it("keeps capture mode independent and preserves precise decimal identities", () => {
    expect(parsePluginPanel(capture).combat).toBeNull();
    expect(parsePluginPanel(connected).combat?.hits).toBe("9007199254740993");
    expect(parsePluginPanel(connected).combat?.totalDamage).toBe(123.456789);
  });
  it("rejects missing fields, traversal, unknown states, oversized and stale data", () => {
    for (const value of [
      null,
      {},
      { ...capture, contractVersion: 2 },
      { ...connected, mode: "packet_capture" },
      { ...connected, connection: "unavailable" },
      { ...connected, combat: null },
      { ...capture, plugins: [{ file: "../a.dll", state: "loaded" }] },
      {
        ...connected,
        plugins: Array(65).fill({ file: "x.dll", state: "loaded" }),
      },
      {
        ...connected,
        combat: { ...connected.combat, hits: Number("9007199254740993") },
      },
      { ...connected, combat: { ...connected.combat, totalDamage: NaN } },
    ])
      expect(() => parsePluginPanel(value)).toThrow();
  });
  it("submits explicit risk consent and never retries a timed-out mutation", async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(capture)
      .mockRejectedValueOnce({ code: "plugin_timeout" });
    const client = createToolkitClient({ invoke });
    await client.setMode("plugin", true);
    expect(invoke).toHaveBeenNthCalledWith(1, "set_data_mode", {
      mode: "plugin",
      acknowledgeRisk: true,
    });
    await expect(
      client.control("reload", "NTE_PluginCombat.dll"),
    ).rejects.toEqual({ code: "plugin_timeout" });
    expect(invoke).toHaveBeenCalledTimes(2);
  });
});
