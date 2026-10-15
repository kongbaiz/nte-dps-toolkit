import { describe, it, expect } from "vitest";
import {
  RELEASE_ACTIONS,
  requiresConfirmation,
} from "@/lib/tauri/release-plugin-client";
import { RELEASE_GROUPS, pluginControlGroup } from "./release-control-groups";
describe("plugin function groups", () => {
  it("preserves every Release action exactly once", () => {
    const grouped = RELEASE_GROUPS.flatMap((g) => [
      ...g.actions,
      ...g.advanced,
    ]);
    expect(new Set(grouped).size).toBe(grouped.length);
    expect([...grouped].sort()).toEqual(
      RELEASE_ACTIONS.map((a) => a[0])
        .filter(
          (a) =>
            ![
              "combatStopExport",
              "hudStatus",
              "hudConfigure",
              "networkStatus",
              "networkEnable",
              "networkFlush",
            ].includes(a),
        )
        .sort(),
    );
  });
  it("assigns runtime snapshot, radar and tracing only to the combat plugin", () => {
    const combat = pluginControlGroup("NTE_PluginCombat.dll")!;
    const host = RELEASE_GROUPS.find((group) => group.id === "host")!;
    expect([...combat.actions, ...combat.advanced]).toEqual(
      expect.arrayContaining([
        "runtimeRefresh",
        "radarRefresh",
        "traceEnable",
        "traceClear",
      ]),
    );
    expect([...host.actions, ...host.advanced].sort()).toEqual([
      "hostStatus",
      "shutdown",
    ]);
    expect(pluginControlGroup("nte_plugincombat.DLL")).toBe(combat);
    expect(pluginControlGroup("Custom.dll")).toBeUndefined();
    expect(pluginControlGroup("NTE_PluginNetwork.dll")).toBeUndefined();
    expect(pluginControlGroup("NTE_PluginPerformance.dll")).toBeUndefined();
  });
  it("keeps destructive maintenance out of primary plugin features", () => {
    expect(RELEASE_GROUPS.find((g) => g.id === "combat")?.advanced).toContain(
      "combatReset",
    );
    expect(RELEASE_GROUPS.find((g) => g.id === "account")?.advanced).toContain(
      "userCancel",
    );
    expect(requiresConfirmation("combatReset")).toBe(true);
    expect(RELEASE_GROUPS.find((g) => g.id === "host")?.actions).toEqual([
      "hostStatus",
      "shutdown",
    ]);
  });
});
