import { describe, expect, it } from "vitest";

import { parseDpsTimeRuntime } from "@/lib/tauri/dps-time-contract";

describe("DPS time runtime contract", () => {
  it("accepts authoritative adjusted and explicit degraded wall-clock states", () => {
    expect(
      parseDpsTimeRuntime({
        configuredMode: "time-stop-adjusted",
        effectiveMode: "time-stop-adjusted",
        combatClockHealth: "recorded",
        degraded: false,
        warningMessageKey: null,
      }).effectiveMode,
    ).toBe("time-stop-adjusted");
    expect(
      parseDpsTimeRuntime({
        configuredMode: "time-stop-adjusted",
        effectiveMode: "real-time",
        combatClockHealth: "provider-unavailable",
        degraded: true,
        warningMessageKey:
          "Time-stop adjustment is unavailable because the combat-clock provider is not connected.",
      }).degraded,
    ).toBe(true);
    expect(
      parseDpsTimeRuntime({
        configuredMode: "time-stop-adjusted",
        effectiveMode: "real-time",
        combatClockHealth: "data-unavailable",
        degraded: true,
        warningMessageKey:
          "Time-stop adjustment is unavailable because the combat-clock provider has no authoritative pause state.",
      }).combatClockHealth,
    ).toBe("data-unavailable");
  });

  it("rejects a configured adjusted label that silently claims unavailable data", () => {
    expect(() =>
      parseDpsTimeRuntime({
        configuredMode: "time-stop-adjusted",
        effectiveMode: "time-stop-adjusted",
        combatClockHealth: "provider-unavailable",
        degraded: false,
        warningMessageKey: null,
      }),
    ).toThrow(/inconsistent configured\/effective runtime state/);
  });

  it("rejects warnings attached to an explicitly real-time mode", () => {
    expect(() =>
      parseDpsTimeRuntime({
        configuredMode: "real-time",
        effectiveMode: "real-time",
        combatClockHealth: "mod-disabled",
        degraded: true,
        warningMessageKey:
          "Time-stop adjustment is unavailable because the required Mod is disabled.",
      }),
    ).toThrow(/inconsistent configured\/effective runtime state/);
  });
});
