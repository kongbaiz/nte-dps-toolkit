import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";

export const DPS_TIME_WARNING_KEYS = [
  "Time-stop adjustment has not been verified for this session.",
  "Time-stop adjustment is unavailable because the combat-clock provider is not connected.",
  "Time-stop adjustment is unavailable because the required Mod is disabled.",
  "Time-stop adjustment is unavailable because the combat-clock provider has no authoritative pause state.",
  "Time-stop adjustment is unavailable because the combat-clock response is invalid.",
] as const;

const DEGRADED_WARNING_BY_HEALTH = {
  unknown: DPS_TIME_WARNING_KEYS[0],
  "provider-unavailable": DPS_TIME_WARNING_KEYS[1],
  "mod-disabled": DPS_TIME_WARNING_KEYS[2],
  "data-unavailable": DPS_TIME_WARNING_KEYS[3],
  "invalid-response": DPS_TIME_WARNING_KEYS[4],
} as const;

const { boolean, enumValue, nullableEnumValue, record } =
  createContractPrimitives((message) => {
    throw new TechnicalContractError(message);
  });

export type DpsTimeModeId = "time-stop-adjusted" | "real-time";
export type DpsTimeEffectiveMode = DpsTimeModeId;
export type CombatClockHealth =
  | "unknown"
  | "recorded"
  | "available"
  | "provider-unavailable"
  | "mod-disabled"
  | "data-unavailable"
  | "invalid-response";

export interface DpsTimeRuntime {
  configuredMode: DpsTimeModeId;
  effectiveMode: DpsTimeEffectiveMode;
  combatClockHealth: CombatClockHealth;
  degraded: boolean;
  warningMessageKey: (typeof DPS_TIME_WARNING_KEYS)[number] | null;
}

export function parseDpsTimeRuntime(
  value: unknown,
  field = "dpsTime",
): DpsTimeRuntime {
  const runtime = record(value, field);
  const parsed: DpsTimeRuntime = {
    configuredMode: enumValue(
      runtime.configuredMode,
      ["time-stop-adjusted", "real-time"] as const,
      `${field}.configuredMode`,
    ),
    effectiveMode: enumValue(
      runtime.effectiveMode,
      ["time-stop-adjusted", "real-time"] as const,
      `${field}.effectiveMode`,
    ),
    combatClockHealth: enumValue(
      runtime.combatClockHealth,
      [
        "unknown",
        "recorded",
        "available",
        "provider-unavailable",
        "mod-disabled",
        "data-unavailable",
        "invalid-response",
      ] as const,
      `${field}.combatClockHealth`,
    ),
    degraded: boolean(runtime.degraded, `${field}.degraded`),
    warningMessageKey: nullableEnumValue(
      runtime.warningMessageKey,
      DPS_TIME_WARNING_KEYS,
      `${field}.warningMessageKey`,
    ),
  };
  const providerReady =
    parsed.combatClockHealth === "available" ||
    parsed.combatClockHealth === "recorded";
  const adjustedEffective = parsed.effectiveMode === "time-stop-adjusted";
  const warningPresent = parsed.warningMessageKey !== null;
  const expectedDegradedWarning = providerReady
    ? null
    : DEGRADED_WARNING_BY_HEALTH[
        parsed.combatClockHealth as keyof typeof DEGRADED_WARNING_BY_HEALTH
      ];
  const valid =
    parsed.configuredMode === "real-time"
      ? parsed.effectiveMode === "real-time" &&
        !parsed.degraded &&
        !warningPresent
      : providerReady
        ? adjustedEffective && !parsed.degraded && !warningPresent
        : !adjustedEffective &&
          parsed.degraded &&
          warningPresent &&
          parsed.warningMessageKey === expectedDegradedWarning;
  if (!valid) {
    throw new TechnicalContractError(
      `${field} has an inconsistent configured/effective runtime state`,
    );
  }
  return parsed;
}
