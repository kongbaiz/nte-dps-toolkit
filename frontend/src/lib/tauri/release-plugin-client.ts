import { tauriInvokeTransport, type InvokeTransport } from "./stream-client";

export const RELEASE_ACTIONS = [
  ["hostStatus", 2, "Host status"],
  ["shutdown", 11, "Unload host"],
  ["combatStatus", 100, "Collection status"],
  ["combatReset", 103, "Reset native combat report"],
  ["combatExport", 104, "Export native combat report"],
  ["combatStopExport", 105, "Stop and export native report"],
  ["combatOperation", 106, "Combat operation status"],
  ["combatReport", 107, "Preview native combat report"],
  ["evidenceStatus", 108, "Evidence recording status"],
  ["networkStatus", 109, "Plugin network status"],
  ["networkEnable", 110, "Plugin network recording"],
  ["networkFlush", 113, "Flush plugin network evidence"],
  ["runtimeRefresh", 114, "Refresh plugin runtime snapshot"],
  ["radarRefresh", 115, "Request plugin radar refresh"],
  ["traceEnable", 116, "Plugin change tracing"],
  ["traceClear", 117, "Clear plugin change trace"],
  ["hudStatus", 118, "In-game HUD status"],
  ["hudConfigure", 119, "In-game enhanced HUD"],
  ["prebattleSkillStatus", 120, "Prebattle skill unlock status"],
  ["prebattleSkillUnlock", 121, "Unlock prebattle skills"],
  ["userStatus", 300, "Account reader status"],
  ["userRefresh", 301, "Refresh account data"],
  ["userOperation", 302, "Account operation status"],
  ["userSnapshot", 303, "Preview account snapshot"],
  ["userExport", 304, "Export account snapshot"],
  ["userCancel", 305, "Cancel account refresh"],
] as const;
export type ReleaseAction = (typeof RELEASE_ACTIONS)[number][0];
export function parseHudOptions(result: ReleaseResult): number {
  if (![118, 119].includes(result.command) || result.truncated)
    throw new TypeError("invalid HUD result");
  const value: unknown = JSON.parse(result.preview);
  if (
    typeof value !== "object" ||
    value === null ||
    !("options" in value) ||
    typeof value.options !== "number" ||
    !Number.isInteger(value.options) ||
    value.options < 0 ||
    value.options > 31
  )
    throw new TypeError("invalid HUD options");
  return value.options;
}
export interface ReleaseResult {
  command: number;
  preview: string;
  totalBytes: number;
  truncated: boolean;
}
export function parseReleaseResult(v: unknown, command: number): ReleaseResult {
  if (typeof v !== "object" || v === null)
    throw new TypeError("invalid release result");
  const r = v as Record<string, unknown>;
  if (
    r.command !== command ||
    typeof r.preview !== "string" ||
    new TextEncoder().encode(r.preview).length > 8192 ||
    typeof r.totalBytes !== "number" ||
    !Number.isSafeInteger(r.totalBytes) ||
    r.totalBytes < 0 ||
    r.totalBytes > 33554432 ||
    typeof r.truncated !== "boolean"
  )
    throw new TypeError("invalid release result");
  const length = new TextEncoder().encode(r.preview).length;
  if (length > r.totalBytes || r.truncated !== length < r.totalBytes)
    throw new TypeError("inconsistent release preview");
  return {
    command,
    preview: r.preview,
    totalBytes: r.totalBytes,
    truncated: r.truncated,
  };
}
export function requiresIdle(a: ReleaseAction) {
  return ["combatReset", "combatStopExport", "shutdown"].includes(a);
}
export function requiresConfirmation(a: ReleaseAction) {
  return [
    "combatReset",
    "combatStopExport",
    "shutdown",
    "traceClear",
    "userCancel",
    "prebattleSkillUnlock",
  ].includes(a);
}
export function createReleasePluginClient(
  transport: InvokeTransport = tauriInvokeTransport,
) {
  return {
    async execute(
      action: ReleaseAction,
      expectedIdentity: string,
      value: number | null = null,
      confirmed = false,
    ) {
      const descriptor = RELEASE_ACTIONS.find((a) => a[0] === action);
      if (!descriptor) throw new TypeError("unknown release action");
      return parseReleaseResult(
        await transport.invoke("release_plugin_action", {
          action,
          expectedIdentity,
          value,
          confirmed,
        }),
        descriptor[1],
      );
    },
  };
}
export const releasePluginClient = createReleasePluginClient();
