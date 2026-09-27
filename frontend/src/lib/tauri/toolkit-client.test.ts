import { describe, expect, it, vi } from "vitest";
import { createToolkitClient, parsePluginPanel } from "./toolkit-client";

const capture = {
  contractVersion: 4,
  loadingMethod: "proxy",
  connectionIdentity: null,
  collectorActive: false,
  capabilities: [],
  mode: "packet_capture",
  connection: "notRequested",
  componentsReady: false,
  plugins: [],
  combat: null,
  operation: null,
};
const connected = {
  ...capture,
  mode: "plugin",
  connection: "connected",
  connectionIdentity: "123:456",
  capabilities: [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 100, 101, 102, 103, 104, 105, 106, 107,
    108, 109, 110, 113, 114, 115, 116, 117, 200, 300, 301, 302, 303, 304, 305,
  ],
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
      { ...capture, contractVersion: 1 },
      { ...connected, mode: "packet_capture" },
      { ...connected, connection: "unavailable" },
      { ...connected, connectionIdentity: null },
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
      client.control("disable", "NTE_PluginCombat.dll"),
    ).rejects.toEqual({ code: "plugin_timeout" });
    expect(invoke).toHaveBeenCalledTimes(2);
  });
});

it("keeps the host controllable when Combat is unloaded and rejects invalid capabilities", () => {
  expect(parsePluginPanel({ ...connected, combat: null }).connection).toBe(
    "connected",
  );
  for (const capabilities of [[1, 1], [-1], [1.5], Array(129).fill(1)]) {
    expect(() => parsePluginPanel({ ...connected, capabilities })).toThrow();
  }
});

it("routes loading preference and explicit host startup separately from data mode", async () => {
  const invoke = vi
    .fn()
    .mockResolvedValue({ ...capture, loadingMethod: "loader" });
  const client = createToolkitClient({ invoke });
  expect((await client.setLoadingMethod("loader")).loadingMethod).toBe(
    "loader",
  );
  expect(invoke).toHaveBeenLastCalledWith("set_host_loading_method", {
    method: "loader",
  });
  invoke.mockResolvedValueOnce({
    panel: { ...capture, loadingMethod: "loader" },
    outcome: "connected",
  });
  expect((await client.launchHost()).outcome).toBe("connected");
  expect(invoke).toHaveBeenLastCalledWith("launch_plugin_host");
  expect(() =>
    parsePluginPanel({ ...capture, loadingMethod: undefined }),
  ).toThrow();
  expect(() =>
    parsePluginPanel({ ...capture, loadingMethod: "unknown" }),
  ).toThrow();
});
