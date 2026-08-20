import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import {
  parseStreamSubscriptionReceipt,
  type StreamSubscriptionReceipt,
} from "@/lib/tauri/stream-contract";

export { HUD_WINDOW_LABEL } from "@/lib/tauri/window-labels";
export const TECHNICAL_CONTRACT_VERSION = 5;
export const HUD_SNAPSHOT_VERSION = 3;
export const HUD_TIMELINE_MAX_BUCKETS = 60;
export const HUD_MODULE_IDS = [
  "title",
  "summary",
  "status",
  "characters",
  "timeline",
] as const;

const {
  array,
  boolean,
  digitString: decimalString,
  integer,
  isRecord,
  nonNegativeNumber: finiteNumber,
  nullableInteger,
  nullableString,
  positiveNumber: positiveFiniteNumber,
  record,
  string,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

export type HudModuleId = (typeof HUD_MODULE_IDS)[number];

export interface HudWindowSnapshot {
  passthrough: boolean;
  alwaysOnTop: boolean;
}

export interface CaptureIssueSnapshot {
  code: string;
  messageKey: string;
  messageArguments: string[];
}

export interface CaptureSnapshot {
  phase: string;
  messageKey: string;
  messageArguments: string[];
  issue: CaptureIssueSnapshot | null;
}

export interface HudConfigSnapshot {
  width: number;
  moduleOrder: string[];
  showTitle: boolean;
  showTeamDps: boolean;
  showDuration: boolean;
  showTotalDamage: boolean;
  showCharacterRows: boolean;
  showDamageTaken: boolean;
  showAbyssHalf: boolean;
  showPassthroughState: boolean;
  showMiniTimeline: boolean;
}

export interface HudSummarySnapshot {
  teamDps: number;
  durationSeconds: number;
  totalDamage: number;
  totalDamageTaken: number;
}

export interface HudCharacterSnapshot {
  characterId: number;
  name: string;
  previewLabelSuffix: string | null;
  hits: string;
  damage: number;
  dps: number;
  damageSharePercent: number;
  damageTaken: number;
  color: string | null;
}

export interface HudStatusSnapshot {
  abyssDetected: boolean;
  abyssFloor: number | null;
  abyssHalf: string | null;
  abyssSuccess: boolean;
}

export interface HudTimelineBucketSnapshot {
  startSeconds: number;
  endSeconds: number;
  damage: number;
  dps: number;
  hits: string;
}

export interface HudTimelineSnapshot {
  bucketSeconds: number;
  durationSeconds: number;
  peakDps: number;
  buckets: HudTimelineBucketSnapshot[];
}

export interface HudSnapshot {
  version: number;
  dataState: string;
  config: HudConfigSnapshot;
  summary: HudSummarySnapshot | null;
  characters: HudCharacterSnapshot[];
  status: HudStatusSnapshot;
  timeline: HudTimelineSnapshot | null;
}

export interface TechnicalSnapshot {
  contractVersion: number;
  sequence: string;
  bridgeStatus: string;
  adapterVersion: string;
  windowLabel: string;
  uptimeMs: string;
  streamIntervalMs: number;
  supportedLocales: string[];
  window: HudWindowSnapshot;
  capture: CaptureSnapshot;
  hud: HudSnapshot;
}

export type SubscriptionReceipt = StreamSubscriptionReceipt;

export interface TechnicalCommandError {
  code: string;
  messageKey: string;
  messageArguments: string[];
}

export interface TechnicalSnapshotEvent {
  event: "snapshot";
  payload: TechnicalSnapshot;
}

export type TechnicalEvent = TechnicalSnapshotEvent;

export function parseTechnicalSnapshot(value: unknown): TechnicalSnapshot {
  const snapshot = record(value, "technical snapshot");
  const window = record(snapshot.window, "HUD window snapshot");
  const supportedLocales = array(
    snapshot.supportedLocales,
    "supported locales",
  );
  const contractVersion = integer(snapshot.contractVersion, "contractVersion");

  if (contractVersion !== TECHNICAL_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported technical contract version: ${contractVersion}`,
    );
  }

  return {
    contractVersion,
    sequence: decimalString(snapshot.sequence, "sequence"),
    bridgeStatus: string(snapshot.bridgeStatus, "bridgeStatus"),
    adapterVersion: string(snapshot.adapterVersion, "adapterVersion"),
    windowLabel: string(snapshot.windowLabel, "windowLabel"),
    uptimeMs: decimalString(snapshot.uptimeMs, "uptimeMs"),
    streamIntervalMs: integer(snapshot.streamIntervalMs, "streamIntervalMs"),
    supportedLocales: supportedLocales.map((locale, index) =>
      string(locale, `supportedLocales[${index}]`),
    ),
    window: {
      passthrough: boolean(window.passthrough, "window.passthrough"),
      alwaysOnTop: boolean(window.alwaysOnTop, "window.alwaysOnTop"),
    },
    capture: parseCaptureSnapshot(snapshot.capture),
    hud: parseHudSnapshot(snapshot.hud),
  };
}

export function parseCaptureSnapshot(value: unknown): CaptureSnapshot {
  const capture = record(value, "capture snapshot");
  const messageArguments = array(
    capture.messageArguments,
    "capture.messageArguments",
  );
  return {
    phase: string(capture.phase, "capture.phase"),
    messageKey: string(capture.messageKey, "capture.messageKey"),
    messageArguments: messageArguments.map((argument, index) =>
      string(argument, `capture.messageArguments[${index}]`),
    ),
    issue:
      capture.issue === null
        ? null
        : parseCaptureIssue(capture.issue, "capture.issue"),
  };
}

function parseCaptureIssue(
  value: unknown,
  field: string,
): CaptureIssueSnapshot {
  const issue = record(value, field);
  const messageArguments = array(
    issue.messageArguments,
    `${field}.messageArguments`,
  );
  return {
    code: string(issue.code, `${field}.code`),
    messageKey: string(issue.messageKey, `${field}.messageKey`),
    messageArguments: messageArguments.map((argument, index) =>
      string(argument, `${field}.messageArguments[${index}]`),
    ),
  };
}

export function parseHudSnapshot(value: unknown): HudSnapshot {
  const hud = record(value, "HUD snapshot");
  const version = integer(hud.version, "hud.version");
  if (version !== HUD_SNAPSHOT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported HUD snapshot version: ${version}`,
    );
  }

  const characters = array(hud.characters, "hud.characters");
  const status = record(hud.status, "hud.status");

  return {
    version,
    dataState: string(hud.dataState, "hud.dataState"),
    config: parseHudConfigSnapshot(hud.config, "hud.config"),
    summary:
      hud.summary === null ? null : parseHudSummary(hud.summary, "hud.summary"),
    characters: characters.map((character, index) =>
      parseHudCharacter(character, `hud.characters[${index}]`),
    ),
    status: {
      abyssDetected: boolean(status.abyssDetected, "hud.status.abyssDetected"),
      abyssFloor: nullableInteger(status.abyssFloor, "hud.status.abyssFloor"),
      abyssHalf: nullableString(status.abyssHalf, "hud.status.abyssHalf"),
      abyssSuccess: boolean(status.abyssSuccess, "hud.status.abyssSuccess"),
    },
    timeline:
      hud.timeline === null
        ? null
        : parseHudTimeline(hud.timeline, "hud.timeline"),
  };
}

export function parseHudConfigSnapshot(
  value: unknown,
  field = "hud.config",
): HudConfigSnapshot {
  const config = record(value, field);
  const moduleOrder = array(config.moduleOrder, `${field}.moduleOrder`).map(
    (module, index) => string(module, `${field}.moduleOrder[${index}]`),
  );
  if (
    moduleOrder.length !== HUD_MODULE_IDS.length ||
    new Set(moduleOrder).size !== HUD_MODULE_IDS.length ||
    moduleOrder.some(
      (module) => !HUD_MODULE_IDS.includes(module as HudModuleId),
    )
  ) {
    throw new TechnicalContractError(
      `${field}.moduleOrder must contain every stable HUD module exactly once`,
    );
  }

  return {
    width: integer(config.width, `${field}.width`),
    moduleOrder,
    showTitle: boolean(config.showTitle, `${field}.showTitle`),
    showTeamDps: boolean(config.showTeamDps, `${field}.showTeamDps`),
    showDuration: boolean(config.showDuration, `${field}.showDuration`),
    showTotalDamage: boolean(
      config.showTotalDamage,
      `${field}.showTotalDamage`,
    ),
    showCharacterRows: boolean(
      config.showCharacterRows,
      `${field}.showCharacterRows`,
    ),
    showDamageTaken: boolean(
      config.showDamageTaken,
      `${field}.showDamageTaken`,
    ),
    showAbyssHalf: boolean(config.showAbyssHalf, `${field}.showAbyssHalf`),
    showPassthroughState: boolean(
      config.showPassthroughState,
      `${field}.showPassthroughState`,
    ),
    showMiniTimeline: boolean(
      config.showMiniTimeline,
      `${field}.showMiniTimeline`,
    ),
  };
}

function parseHudSummary(value: unknown, field: string): HudSummarySnapshot {
  const summary = record(value, field);
  return {
    teamDps: finiteNumber(summary.teamDps, `${field}.teamDps`),
    durationSeconds: finiteNumber(
      summary.durationSeconds,
      `${field}.durationSeconds`,
    ),
    totalDamage: finiteNumber(summary.totalDamage, `${field}.totalDamage`),
    totalDamageTaken: finiteNumber(
      summary.totalDamageTaken,
      `${field}.totalDamageTaken`,
    ),
  };
}

function parseHudCharacter(
  value: unknown,
  field: string,
): HudCharacterSnapshot {
  const character = record(value, field);
  return {
    characterId: integer(character.characterId, `${field}.characterId`),
    name: string(character.name, `${field}.name`),
    previewLabelSuffix: nullableString(
      character.previewLabelSuffix,
      `${field}.previewLabelSuffix`,
    ),
    hits: decimalString(character.hits, `${field}.hits`),
    damage: finiteNumber(character.damage, `${field}.damage`),
    dps: finiteNumber(character.dps, `${field}.dps`),
    damageSharePercent: finiteNumber(
      character.damageSharePercent,
      `${field}.damageSharePercent`,
    ),
    damageTaken: finiteNumber(character.damageTaken, `${field}.damageTaken`),
    color: nullableString(character.color, `${field}.color`),
  };
}

function parseHudTimeline(value: unknown, field: string): HudTimelineSnapshot {
  const timeline = record(value, field);
  const buckets = array(timeline.buckets, `${field}.buckets`);
  if (buckets.length > HUD_TIMELINE_MAX_BUCKETS) {
    throw new TechnicalContractError(
      `${field}.buckets exceeds ${HUD_TIMELINE_MAX_BUCKETS} entries`,
    );
  }

  return {
    bucketSeconds: positiveFiniteNumber(
      timeline.bucketSeconds,
      `${field}.bucketSeconds`,
    ),
    durationSeconds: finiteNumber(
      timeline.durationSeconds,
      `${field}.durationSeconds`,
    ),
    peakDps: finiteNumber(timeline.peakDps, `${field}.peakDps`),
    buckets: buckets.map((bucket, index) =>
      parseHudTimelineBucket(bucket, `${field}.buckets[${index}]`),
    ),
  };
}

function parseHudTimelineBucket(
  value: unknown,
  field: string,
): HudTimelineBucketSnapshot {
  const bucket = record(value, field);
  const startSeconds = finiteNumber(
    bucket.startSeconds,
    `${field}.startSeconds`,
  );
  const endSeconds = finiteNumber(bucket.endSeconds, `${field}.endSeconds`);
  if (endSeconds < startSeconds) {
    throw new TechnicalContractError(
      `${field}.endSeconds must not precede startSeconds`,
    );
  }

  return {
    startSeconds,
    endSeconds,
    damage: finiteNumber(bucket.damage, `${field}.damage`),
    dps: finiteNumber(bucket.dps, `${field}.dps`),
    hits: decimalString(bucket.hits, `${field}.hits`),
  };
}

export function parseTechnicalEvent(value: unknown): TechnicalEvent {
  const event = record(value, "technical event");

  if (event.event !== "snapshot") {
    throw new TechnicalContractError("Unknown technical event");
  }

  return {
    event: "snapshot",
    payload: parseTechnicalSnapshot(event.payload),
  };
}

export function parseSubscriptionReceipt(value: unknown): SubscriptionReceipt {
  try {
    return parseStreamSubscriptionReceipt(value);
  } catch (error) {
    throw new TechnicalContractError(
      error instanceof Error ? error.message : "Invalid subscription receipt",
    );
  }
}

export function parseTechnicalCommandError(
  value: unknown,
): TechnicalCommandError {
  const fallback: TechnicalCommandError = {
    code: "unexpected_technical_error",
    messageKey: "Rust bridge unavailable",
    messageArguments: [],
  };

  if (!isRecord(value)) {
    return fallback;
  }

  const messageArguments = value.messageArguments;
  if (
    typeof value.code !== "string" ||
    typeof value.messageKey !== "string" ||
    !Array.isArray(messageArguments) ||
    !messageArguments.every((argument) => typeof argument === "string")
  ) {
    return fallback;
  }

  return {
    code: value.code,
    messageKey: value.messageKey,
    messageArguments,
  };
}

export class TechnicalContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TechnicalContractError";
  }
}
