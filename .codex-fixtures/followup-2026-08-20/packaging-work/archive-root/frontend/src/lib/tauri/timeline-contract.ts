import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const TIMELINE_CONTRACT_VERSION = 3;
export const TIMELINE_MAX_BUCKETS = 10_000;
export const TIMELINE_MAX_CHARACTERS = 256;
export const TIMELINE_MAX_ROLES_PER_BUCKET = 16;
export const TIMELINE_MAX_CHARACTER_NAME_BYTES = 128;
export const TIMELINE_MAX_INTERVALS = 10_000;
export const TIMELINE_MAX_MARKERS = 64;

const {
  boundedArray: boundedList,
  boundedUtf8String,
  boolean: flag,
  cssHex,
  decimalString: decimal,
  enumValue,
  integer,
  nonNegativeInteger,
  nonNegativeNumber: nonNegative,
  positiveNumber: positive,
  record: object,
  string: text,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

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
  effectiveBucketSeconds: number;
  bucketSecondsMin: number;
  bucketSecondsMax: number;
  bucketSecondsStep: number;
  hasData: boolean;
  duration: number;
  totalDamage: number;
  omittedRoleDamage: number;
  omittedRoleHits: string;
  peakDps: number;
  timeStopDuration: number;
  compactedTimeStopIntervals: string;
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
  const buckets = boundedList(
    item.buckets,
    "timeline.buckets",
    TIMELINE_MAX_BUCKETS,
  );
  const bucketSecondsMin = positive(
    item.bucketSecondsMin,
    "timeline.bucketSecondsMin",
  );
  const bucketSecondsMax = positive(
    item.bucketSecondsMax,
    "timeline.bucketSecondsMax",
  );
  const bucketSeconds = positive(item.bucketSeconds, "timeline.bucketSeconds");
  const effectiveBucketSeconds = positive(
    item.effectiveBucketSeconds,
    "timeline.effectiveBucketSeconds",
  );
  if (
    bucketSecondsMin > bucketSecondsMax ||
    bucketSeconds < bucketSecondsMin ||
    bucketSeconds > bucketSecondsMax ||
    effectiveBucketSeconds < bucketSeconds
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
    effectiveBucketSeconds,
    bucketSecondsMin,
    bucketSecondsMax,
    bucketSecondsStep: positive(
      item.bucketSecondsStep,
      "timeline.bucketSecondsStep",
    ),
    hasData: flag(item.hasData, "timeline.hasData"),
    duration: nonNegative(item.duration, "timeline.duration"),
    totalDamage: nonNegative(item.totalDamage, "timeline.totalDamage"),
    omittedRoleDamage: nonNegative(
      item.omittedRoleDamage,
      "timeline.omittedRoleDamage",
    ),
    omittedRoleHits: decimal(item.omittedRoleHits, "timeline.omittedRoleHits"),
    peakDps: nonNegative(item.peakDps, "timeline.peakDps"),
    timeStopDuration: nonNegative(
      item.timeStopDuration,
      "timeline.timeStopDuration",
    ),
    compactedTimeStopIntervals: decimal(
      item.compactedTimeStopIntervals,
      "timeline.compactedTimeStopIntervals",
    ),
    timeStopIntervals: boundedList(
      item.timeStopIntervals,
      "timeline.timeStopIntervals",
      TIMELINE_MAX_INTERVALS,
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
    markers: boundedList(
      item.markers,
      "timeline.markers",
      TIMELINE_MAX_MARKERS,
    ).map((value, index) => {
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
    characters: boundedList(
      item.characters,
      "timeline.characters",
      TIMELINE_MAX_CHARACTERS,
    ).map((value, index) => {
      const row = object(value, `timeline.characters[${index}]`);
      return {
        id: nonNegativeInteger(row.id, `timeline.characters[${index}].id`),
        name: boundedUtf8String(
          row.name,
          `timeline.characters[${index}].name`,
          TIMELINE_MAX_CHARACTER_NAME_BYTES,
        ),
        color: cssHex(row.color, `timeline.characters[${index}].color`),
        totalDamage: nonNegative(
          row.totalDamage,
          `timeline.characters[${index}].totalDamage`,
        ),
      };
    }),
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
        roles: boundedList(
          row.roles,
          `timeline.buckets[${index}].roles`,
          TIMELINE_MAX_ROLES_PER_BUCKET,
        ).map((value, roleIndex) => {
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
        }),
      };
    }),
    segments: boundedList(
      item.segments,
      "timeline.segments",
      TIMELINE_MAX_BUCKETS,
    ).map((value, index) => {
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
