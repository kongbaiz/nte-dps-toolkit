import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const TIMELINE_CONTRACT_VERSION = 2;
export const TIMELINE_MAX_BUCKETS = 20_000;

export type TimelineScope = "all" | "upper" | "lower";
export type TimelineCurveMode = "team" | "characters";
export type TimelineCommandError = TechnicalCommandError;

export interface TimelineBucket {
  start: number;
  end: number;
  teamDps: number;
  damage: number;
  hits: string;
  cumulativeDamage: number;
  roles: Array<{ characterId: number; dps: number }>;
}

export interface TimelineCharacter {
  id: number;
  name: string;
  color: string;
  totalDamage: number;
}

export interface TimelineMarker {
  offset: number;
  labelKey: string;
  kind: "half" | "clear" | "exit";
}

export interface TimelineSnapshot {
  contractVersion: number;
  generation: string;
  scope: TimelineScope;
  viewMode: TimelineCurveMode;
  bucketSeconds: number;
  bucketSecondsMin: number;
  bucketSecondsMax: number;
  bucketSecondsStep: number;
  hasData: boolean;
  duration: number;
  totalDamage: number;
  peakDps: number;
  timeStopDuration: number;
  timeStopIntervals: Array<{ start: number; end: number }>;
  markers: TimelineMarker[];
  characters: TimelineCharacter[];
  buckets: TimelineBucket[];
  segments: Array<{ start: number; end: number; dps: number }>;
}

export function parseTimelineSnapshot(value: unknown): TimelineSnapshot {
  const item = object(value, "timeline snapshot");
  const contractVersion = integer(
    item.contractVersion,
    "timeline.contractVersion",
  );
  if (contractVersion !== TIMELINE_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported timeline contract version: ${contractVersion}`,
    );
  }
  const buckets = list(item.buckets, "timeline.buckets");
  if (buckets.length > TIMELINE_MAX_BUCKETS) {
    throw new TechnicalContractError("timeline.buckets exceeds display bounds");
  }
  const bucketSecondsMin = positive(
    item.bucketSecondsMin,
    "timeline.bucketSecondsMin",
  );
  const bucketSecondsMax = positive(
    item.bucketSecondsMax,
    "timeline.bucketSecondsMax",
  );
  const bucketSeconds = positive(item.bucketSeconds, "timeline.bucketSeconds");
  if (
    bucketSecondsMin > bucketSecondsMax ||
    bucketSeconds < bucketSecondsMin ||
    bucketSeconds > bucketSecondsMax
  ) {
    throw new TechnicalContractError("timeline bucket range is inconsistent");
  }
  return {
    contractVersion,
    generation: decimal(item.generation, "timeline.generation"),
    scope: enumValue(
      item.scope,
      ["all", "upper", "lower"] as const,
      "timeline.scope",
    ),
    viewMode: enumValue(
      item.viewMode,
      ["team", "characters"] as const,
      "timeline.viewMode",
    ),
    bucketSeconds,
    bucketSecondsMin,
    bucketSecondsMax,
    bucketSecondsStep: positive(
      item.bucketSecondsStep,
      "timeline.bucketSecondsStep",
    ),
    hasData: flag(item.hasData, "timeline.hasData"),
    duration: nonNegative(item.duration, "timeline.duration"),
    totalDamage: nonNegative(item.totalDamage, "timeline.totalDamage"),
    peakDps: nonNegative(item.peakDps, "timeline.peakDps"),
    timeStopDuration: nonNegative(
      item.timeStopDuration,
      "timeline.timeStopDuration",
    ),
    timeStopIntervals: list(
      item.timeStopIntervals,
      "timeline.timeStopIntervals",
    ).map((value, index) => {
      const row = object(value, `timeline.timeStopIntervals[${index}]`);
      return {
        start: nonNegative(
          row.start,
          `timeline.timeStopIntervals[${index}].start`,
        ),
        end: nonNegative(row.end, `timeline.timeStopIntervals[${index}].end`),
      };
    }),
    markers: list(item.markers, "timeline.markers").map((value, index) => {
      const row = object(value, `timeline.markers[${index}]`);
      return {
        offset: nonNegative(row.offset, `timeline.markers[${index}].offset`),
        labelKey: text(row.labelKey, `timeline.markers[${index}].labelKey`),
        kind: enumValue(
          row.kind,
          ["half", "clear", "exit"] as const,
          `timeline.markers[${index}].kind`,
        ),
      };
    }),
    characters: list(item.characters, "timeline.characters").map(
      (value, index) => {
        const row = object(value, `timeline.characters[${index}]`);
        return {
          id: nonNegativeInteger(row.id, `timeline.characters[${index}].id`),
          name: text(row.name, `timeline.characters[${index}].name`),
          color: cssHex(row.color, `timeline.characters[${index}].color`),
          totalDamage: nonNegative(
            row.totalDamage,
            `timeline.characters[${index}].totalDamage`,
          ),
        };
      },
    ),
    buckets: buckets.map((value, index) => {
      const row = object(value, `timeline.buckets[${index}]`);
      return {
        start: nonNegative(row.start, `timeline.buckets[${index}].start`),
        end: nonNegative(row.end, `timeline.buckets[${index}].end`),
        teamDps: nonNegative(row.teamDps, `timeline.buckets[${index}].teamDps`),
        damage: nonNegative(row.damage, `timeline.buckets[${index}].damage`),
        hits: decimal(row.hits, `timeline.buckets[${index}].hits`),
        cumulativeDamage: nonNegative(
          row.cumulativeDamage,
          `timeline.buckets[${index}].cumulativeDamage`,
        ),
        roles: list(row.roles, `timeline.buckets[${index}].roles`).map(
          (value, roleIndex) => {
            const role = object(
              value,
              `timeline.buckets[${index}].roles[${roleIndex}]`,
            );
            return {
              characterId: nonNegativeInteger(
                role.characterId,
                `timeline.buckets[${index}].roles[${roleIndex}].characterId`,
              ),
              dps: nonNegative(
                role.dps,
                `timeline.buckets[${index}].roles[${roleIndex}].dps`,
              ),
            };
          },
        ),
      };
    }),
    segments: list(item.segments, "timeline.segments").map((value, index) => {
      const row = object(value, `timeline.segments[${index}]`);
      return {
        start: nonNegative(row.start, `timeline.segments[${index}].start`),
        end: nonNegative(row.end, `timeline.segments[${index}].end`),
        dps: nonNegative(row.dps, `timeline.segments[${index}].dps`),
      };
    }),
  };
}

export function parseTimelineEvent(value: unknown): TimelineSnapshot {
  const item = object(value, "timeline event");
  if (item.event !== "snapshot") {
    throw new TechnicalContractError("Unsupported timeline event");
  }
  return parseTimelineSnapshot(item.payload);
}

export function timelineError(error: unknown): TimelineCommandError {
  return parseTechnicalCommandError(error);
}

function object(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TechnicalContractError(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}
function list(value: unknown, field: string): unknown[] {
  if (!Array.isArray(value))
    throw new TechnicalContractError(`${field} must be an array`);
  return value;
}
function text(value: unknown, field: string): string {
  if (typeof value !== "string")
    throw new TechnicalContractError(`${field} must be a string`);
  return value;
}
function flag(value: unknown, field: string): boolean {
  if (typeof value !== "boolean")
    throw new TechnicalContractError(`${field} must be a boolean`);
  return value;
}
function number(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value))
    throw new TechnicalContractError(`${field} must be finite`);
  return value;
}
function nonNegative(value: unknown, field: string): number {
  const parsed = number(value, field);
  if (parsed < 0)
    throw new TechnicalContractError(`${field} must not be negative`);
  return parsed;
}
function positive(value: unknown, field: string): number {
  const parsed = number(value, field);
  if (parsed <= 0)
    throw new TechnicalContractError(`${field} must be positive`);
  return parsed;
}
function integer(value: unknown, field: string): number {
  const parsed = number(value, field);
  if (!Number.isInteger(parsed))
    throw new TechnicalContractError(`${field} must be an integer`);
  return parsed;
}
function nonNegativeInteger(value: unknown, field: string): number {
  const parsed = integer(value, field);
  if (parsed < 0)
    throw new TechnicalContractError(`${field} must not be negative`);
  return parsed;
}
function decimal(value: unknown, field: string): string {
  const parsed = text(value, field);
  if (!/^(0|[1-9]\d*)$/.test(parsed))
    throw new TechnicalContractError(`${field} must be a decimal string`);
  return parsed;
}
function enumValue<const T extends readonly string[]>(
  value: unknown,
  options: T,
  field: string,
): T[number] {
  if (typeof value !== "string" || !options.includes(value))
    throw new TechnicalContractError(`${field} has an unsupported value`);
  return value as T[number];
}
function cssHex(value: unknown, field: string): string {
  const parsed = text(value, field);
  if (!/^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/.test(parsed))
    throw new TechnicalContractError(`${field} must be a CSS hex color`);
  return parsed;
}
