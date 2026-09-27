import { createContractPrimitives } from "./contract-primitives";
import { tauriInvokeTransport, type InvokeTransport } from "./stream-client";

export type DataMode = "packet_capture" | "plugin";
export interface PluginPanel {
  contractVersion: 4;
  connectionIdentity: string | null;
  collectorActive: boolean;
  capabilities: number[];
  mode: DataMode;
  loadingMethod: "proxy" | "loader";
  connection: "notRequested" | "connected" | "unavailable" | "busy" | "error";
  componentsReady: boolean;
  plugins: {
    file: string;
    state: "loaded" | "unloaded" | "unload_pending" | "failed";
  }[];
  combat: null | {
    generation: string;
    capturing: boolean;
    hits: string;
    totalDamage: number;
    direct: number;
    correlated: number;
    inferred: number;
    unknown: number;
  };
  operation: null | { state: string; error: string };
}
const fail = (message: string): never => {
  throw new TypeError(message);
};
const p = createContractPrimitives(fail);
export function parsePluginPanel(value: unknown): PluginPanel {
  const r = p.record(value, "pluginPanel");
  if (r.contractVersion !== 4) fail("unsupported plugin panel contract");
  const mode = p.enumValue(
    r.mode,
    ["packet_capture", "plugin"] as const,
    "mode",
  );
  const connection = p.enumValue(
    r.connection,
    ["notRequested", "connected", "unavailable", "busy", "error"] as const,
    "connection",
  );
  const identity =
    r.connectionIdentity === null
      ? null
      : p.boundedString(r.connectionIdentity, "connectionIdentity", 64);
  if (identity !== null && !/^\d+:\d+$/.test(identity))
    fail("invalid host identity");
  const rawCapabilities = p.array(r.capabilities, "capabilities");
  if (rawCapabilities.length > 128) fail("too many capabilities");
  const capabilities = rawCapabilities.map((v) => {
    if (
      typeof v !== "number" ||
      !Number.isSafeInteger(v) ||
      v < 1 ||
      v > 0xffffffff
    )
      return fail("invalid capability");
    return v;
  });
  if (new Set(capabilities).size !== capabilities.length)
    fail("duplicate capability");
  const list = p.array(r.plugins, "plugins");
  if (list.length > 64) fail("too many plugins");
  const files = new Set<string>();
  const plugins = list.map((value) => {
    const item = p.record(value, "plugin");
    const file = p.boundedString(item.file, "file", 128);
    if (
      !/^[A-Za-z0-9_.-]+\.dll$/.test(file) ||
      file.includes("..") ||
      files.has(file.toLowerCase())
    )
      fail("invalid plugin filename");
    files.add(file.toLowerCase());
    return {
      file,
      state: p.enumValue(
        item.state,
        ["loaded", "unloaded", "unload_pending", "failed"] as const,
        "state",
      ),
    };
  });
  let combat: PluginPanel["combat"] = null;
  if (r.combat !== null) {
    const c = p.record(r.combat, "combat");
    const amount = (key: string) => {
      const v = c[key];
      if (typeof v !== "number" || !Number.isFinite(v) || v < 0)
        return fail(`invalid ${key}`);
      return v;
    };
    combat = {
      generation: p.decimalString(c.generation, "generation"),
      capturing: p.boolean(c.capturing, "capturing"),
      hits: p.decimalString(c.hits, "hits"),
      totalDamage: amount("totalDamage"),
      direct: amount("direct"),
      correlated: amount("correlated"),
      inferred: amount("inferred"),
      unknown: amount("unknown"),
    };
  }
  let operation: PluginPanel["operation"] = null;
  if (r.operation !== null) {
    const o = p.record(r.operation, "operation");
    operation = {
      state: p.boundedString(o.state, "state", 64, { allowEmpty: true }),
      error: p.boundedString(o.error, "error", 512, { allowEmpty: true }),
    };
  }
  if (
    connection !== "connected" &&
    (plugins.length || combat || operation || identity || capabilities.length)
  )
    fail("disconnected panel contains live data");
  if (mode === "packet_capture" && connection !== "notRequested")
    fail("capture mode must not use plugin IPC");
  if (connection === "connected" && (operation === null || identity === null))
    fail("missing connected state");
  return {
    contractVersion: 4,
    loadingMethod: p.enumValue(
      r.loadingMethod,
      ["proxy", "loader"] as const,
      "loadingMethod",
    ),
    connectionIdentity: identity,
    collectorActive: p.boolean(r.collectorActive, "collectorActive"),
    capabilities,
    mode,
    connection,
    componentsReady: p.boolean(r.componentsReady, "componentsReady"),
    plugins,
    combat,
    operation,
  };
}
export function createToolkitClient(
  transport: InvokeTransport = tauriInvokeTransport,
) {
  return {
    async snapshot() {
      return parsePluginPanel(await transport.invoke("get_plugin_panel"));
    },
    async setMode(mode: DataMode, acknowledgeRisk: boolean) {
      return parsePluginPanel(
        await transport.invoke("set_data_mode", { mode, acknowledgeRisk }),
      );
    },
    async setLoadingMethod(method: "proxy" | "loader") {
      return parsePluginPanel(
        await transport.invoke("set_host_loading_method", { method }),
      );
    },
    async launchHost() {
      const result = p.record(
        await transport.invoke("launch_plugin_host"),
        "hostLaunch",
      );
      return {
        panel: parsePluginPanel(result.panel),
        outcome: p.enumValue(
          result.outcome,
          ["proxyDeployed", "connected"] as const,
          "outcome",
        ),
      };
    },
    async control(
      action: "enable" | "disable",
      file: string | null = null,
      value: number | null = null,
    ) {
      await transport.invoke("control_plugin", { action, file, value });
    },
  };
}
export const toolkitClient = createToolkitClient();
