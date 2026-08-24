import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import {
  parseCaptureSnapshot,
  parseTechnicalCommandError,
  TechnicalContractError,
  type CaptureSnapshot,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";
import {
  parseDpsTimeRuntime,
  type DpsTimeRuntime,
} from "@/lib/tauri/dps-time-contract";
import {
  MAIN_DPS_ATTRIBUTION_IDS,
  MAIN_DPS_METRIC_IDS,
  type MainDpsAttributionId,
  type MainDpsMetricId,
} from "@/lib/tauri/settings-contract";

export const MAIN_DPS_CONTRACT_VERSION = 8;
export const MAIN_DPS_MAX_HISTORY_RECORDS = 200;
export const MAIN_DPS_MAX_ROUNDS = MAIN_DPS_MAX_HISTORY_RECORDS + 1;
export const MAIN_DPS_MAX_CHARACTERS = 4;
export const MAIN_DPS_MAX_TEXT_BYTES = 256;
export const MAIN_DPS_MAX_PROJECTED_TEXT_BYTES = 128 * 1024;

const {
  array: list,
  boolean,
  boundedArray: boundedList,
  boundedUtf8StringAllowEmpty: boundedText,
  enumValue: oneOf,
  finiteNumber: finite,
  integer,
  nullableInteger,
  nullableBoundedUtf8StringAllowEmpty: nullableBoundedText,
  record: object,
  u64DecimalString: u64DecimalText,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

export interface MainDpsSnapshot {
  contractVersion: number;
  generation: string;
  captureGeneration: string;
  presentationGeneration: string;
  historyGeneration: string;
  adapterVersion: string;
  capture: CaptureSnapshot;
  dpsTime: DpsTimeRuntime;
  processingPaused: boolean;
  pausedPendingEvents: string;
  pausedDebugPackets: string;
  replayRunning: boolean;
  alwaysOnTop: boolean;
  passthrough: boolean;
  appearance: MainDpsAppearance;
  display: MainDpsDisplay;
  rounds: MainDpsRound[];
  selectedRoundId: string | null;
  readout: MainDpsReadout;
  actions: MainDpsActions;
  gameDetected: boolean;
  gameDetectionStatus: GameDetectionStatus;
  hasLiveSessionData: boolean;
  onboarding: MainDpsOnboarding;
  textTruncated: boolean;
}

export type GameDetectionStatus = "running" | "notRunning" | "probeFailed";

export interface MainDpsAppearance {
  language: "en" | "ja" | "zh-CN";
  darkMode: boolean;
  themePreset: "zinc" | "tactical" | "high-contrast";
  accent: "zinc" | "blue" | "violet" | "orange" | "green";
  density: "compact" | "cozy" | "comfortable";
  reduceMotion: boolean;
  opacity: number;
}

export interface MainDpsDisplay {
  metrics: MainDpsMetricId[];
  attributions: MainDpsAttributionId[];
}

export interface MainDpsOnboarding {
  done: boolean;
  step: number;
  captureDeviceCount: number;
  captureDevicesAvailable: boolean;
  gameDetected: boolean;
  gameDetectionStatus: GameDetectionStatus;
  passthroughHotkeyLabel: string;
  passthroughHotkeyReady: boolean;
}

export interface MainDpsRound {
  id: string | null;
  live: boolean;
  displayTime: string | null;
  abyssFloor: number | null;
}

export interface MainDpsReadout {
  dataState: "empty" | "preview" | "live";
  summary: MainDpsSummary;
  characters: MainDpsCharacter[];
  damageAttribution: MainDpsDamageAttribution;
  abyss: {
    detected: boolean;
    floor: number | null;
    half: "first" | "second" | null;
    success: boolean;
  };
}

export interface MainDpsSummary {
  teamDps: number;
  durationSeconds: number;
  totalDamage: number;
  totalDamageTaken: number;
}

export interface MainDpsCharacter {
  characterId: number;
  name: string;
  hits: string;
  damage: number;
  dps: number;
  damageSharePercent: number;
  damageTaken: number;
  durationSeconds: number;
  color: string | null;
  attribute: string | null;
}

export interface MainDpsDamageAttribution {
  totalDamage: number;
  maxHpReduction: number;
  characterDirectDamage: number;
  characterReactionDamage: number;
  sharedDamage: number;
  unattributedDamage: number;
  separateReactionDamage: boolean;
  includeMaxHpReductionInTotalDamage: boolean;
}

export interface MainDpsActions {
  canStartCapture: boolean;
  canStopCapture: boolean;
  canReset: boolean;
  canStartNewRound: boolean;
  canPause: boolean;
  canResume: boolean;
  canImportReplay: boolean;
  characterDetailsAvailable: boolean;
  teamDetailsAvailable: boolean;
  startCaptureRequiresConfirmation: boolean;
  resetRequiresConfirmation: boolean;
  importReplayRequiresConfirmation: boolean;
}

export interface MainDpsResetResult {
  snapshot: MainDpsSnapshot;
  undoToken: string | null;
}

export type MainDpsCommandError = TechnicalCommandError;

export function parseMainDpsSnapshot(value: unknown): MainDpsSnapshot {
  const source = object(value, "main DPS snapshot");
  const version = integer(source.contractVersion, "contractVersion");
  if (version !== MAIN_DPS_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported main DPS contract: ${version}`,
    );
  }
  const readout = object(source.readout, "readout");
  const summary = object(readout.summary, "readout.summary");
  const abyss = object(readout.abyss, "readout.abyss");
  const damageAttribution = object(
    readout.damageAttribution,
    "readout.damageAttribution",
  );
  const appearance = object(source.appearance, "appearance");
  const display = object(source.display, "display");
  const actions = object(source.actions, "actions");
  const onboarding = object(source.onboarding, "onboarding");
  const dataState = oneOf(
    readout.dataState,
    ["empty", "preview", "live"],
    "readout.dataState",
  );
  const rounds = parseRounds(source.rounds);

  const snapshot: MainDpsSnapshot = {
    contractVersion: version,
    generation: u64DecimalText(source.generation, "generation"),
    captureGeneration: u64DecimalText(
      source.captureGeneration,
      "captureGeneration",
    ),
    presentationGeneration: u64DecimalText(
      source.presentationGeneration,
      "presentationGeneration",
    ),
    historyGeneration: u64DecimalText(
      source.historyGeneration,
      "historyGeneration",
    ),
    adapterVersion: boundedText(source.adapterVersion, "adapterVersion", 128),
    capture: parseCaptureSnapshot(source.capture),
    dpsTime: parseDpsTimeRuntime(source.dpsTime),
    processingPaused: boolean(source.processingPaused, "processingPaused"),
    pausedPendingEvents: u64DecimalText(
      source.pausedPendingEvents,
      "pausedPendingEvents",
    ),
    pausedDebugPackets: u64DecimalText(
      source.pausedDebugPackets,
      "pausedDebugPackets",
    ),
    replayRunning: boolean(source.replayRunning, "replayRunning"),
    alwaysOnTop: boolean(source.alwaysOnTop, "alwaysOnTop"),
    passthrough: boolean(source.passthrough, "passthrough"),
    appearance: {
      language: oneOf(
        appearance.language,
        ["en", "ja", "zh-CN"],
        "appearance.language",
      ),
      darkMode: boolean(appearance.darkMode, "appearance.darkMode"),
      themePreset: oneOf(
        appearance.themePreset,
        ["zinc", "tactical", "high-contrast"],
        "appearance.themePreset",
      ),
      accent: oneOf(
        appearance.accent,
        ["zinc", "blue", "violet", "orange", "green"],
        "appearance.accent",
      ),
      density: oneOf(
        appearance.density,
        ["compact", "cozy", "comfortable"],
        "appearance.density",
      ),
      reduceMotion: boolean(appearance.reduceMotion, "appearance.reduceMotion"),
      opacity: finite(appearance.opacity, "appearance.opacity"),
    },
    display: {
      metrics: parseUniqueIds(
        display.metrics,
        MAIN_DPS_METRIC_IDS,
        "display.metrics",
      ),
      attributions: parseUniqueIds(
        display.attributions,
        MAIN_DPS_ATTRIBUTION_IDS,
        "display.attributions",
      ),
    },
    rounds,
    selectedRoundId: nullableBoundedText(
      source.selectedRoundId,
      "selectedRoundId",
      MAIN_DPS_MAX_TEXT_BYTES,
    ),
    readout: {
      dataState,
      summary: {
        teamDps: finite(summary.teamDps, "summary.teamDps"),
        durationSeconds: finite(
          summary.durationSeconds,
          "summary.durationSeconds",
        ),
        totalDamage: finite(summary.totalDamage, "summary.totalDamage"),
        totalDamageTaken: finite(
          summary.totalDamageTaken,
          "summary.totalDamageTaken",
        ),
      },
      characters: boundedList(
        readout.characters,
        "readout.characters",
        MAIN_DPS_MAX_CHARACTERS,
      ).map(parseCharacter),
      damageAttribution: {
        totalDamage: finite(
          damageAttribution.totalDamage,
          "damageAttribution.totalDamage",
        ),
        maxHpReduction: finite(
          damageAttribution.maxHpReduction,
          "damageAttribution.maxHpReduction",
        ),
        characterDirectDamage: finite(
          damageAttribution.characterDirectDamage,
          "damageAttribution.characterDirectDamage",
        ),
        characterReactionDamage: finite(
          damageAttribution.characterReactionDamage,
          "damageAttribution.characterReactionDamage",
        ),
        sharedDamage: finite(
          damageAttribution.sharedDamage,
          "damageAttribution.sharedDamage",
        ),
        unattributedDamage: finite(
          damageAttribution.unattributedDamage,
          "damageAttribution.unattributedDamage",
        ),
        separateReactionDamage: boolean(
          damageAttribution.separateReactionDamage,
          "damageAttribution.separateReactionDamage",
        ),
        includeMaxHpReductionInTotalDamage: boolean(
          damageAttribution.includeMaxHpReductionInTotalDamage,
          "damageAttribution.includeMaxHpReductionInTotalDamage",
        ),
      },
      abyss: {
        detected: boolean(abyss.detected, "abyss.detected"),
        floor: nullableInteger(abyss.floor, "abyss.floor"),
        half:
          abyss.half === null
            ? null
            : oneOf(abyss.half, ["first", "second"], "abyss.half"),
        success: boolean(abyss.success, "abyss.success"),
      },
    },
    actions: {
      canStartCapture: boolean(
        actions.canStartCapture,
        "actions.canStartCapture",
      ),
      canStopCapture: boolean(actions.canStopCapture, "actions.canStopCapture"),
      canReset: boolean(actions.canReset, "actions.canReset"),
      canStartNewRound: boolean(
        actions.canStartNewRound,
        "actions.canStartNewRound",
      ),
      canPause: boolean(actions.canPause, "actions.canPause"),
      canResume: boolean(actions.canResume, "actions.canResume"),
      canImportReplay: boolean(
        actions.canImportReplay,
        "actions.canImportReplay",
      ),
      characterDetailsAvailable: boolean(
        actions.characterDetailsAvailable,
        "actions.characterDetailsAvailable",
      ),
      teamDetailsAvailable: boolean(
        actions.teamDetailsAvailable,
        "actions.teamDetailsAvailable",
      ),
      startCaptureRequiresConfirmation: boolean(
        actions.startCaptureRequiresConfirmation,
        "actions.startCaptureRequiresConfirmation",
      ),
      resetRequiresConfirmation: boolean(
        actions.resetRequiresConfirmation,
        "actions.resetRequiresConfirmation",
      ),
      importReplayRequiresConfirmation: boolean(
        actions.importReplayRequiresConfirmation,
        "actions.importReplayRequiresConfirmation",
      ),
    },
    gameDetected: boolean(source.gameDetected, "gameDetected"),
    gameDetectionStatus: oneOf(
      source.gameDetectionStatus,
      ["running", "notRunning", "probeFailed"] as const,
      "gameDetectionStatus",
    ),
    hasLiveSessionData: boolean(
      source.hasLiveSessionData,
      "hasLiveSessionData",
    ),
    onboarding: {
      done: boolean(onboarding.done, "onboarding.done"),
      step: integer(onboarding.step, "onboarding.step"),
      captureDeviceCount: integer(
        onboarding.captureDeviceCount,
        "onboarding.captureDeviceCount",
      ),
      captureDevicesAvailable: boolean(
        onboarding.captureDevicesAvailable,
        "onboarding.captureDevicesAvailable",
      ),
      gameDetected: boolean(onboarding.gameDetected, "onboarding.gameDetected"),
      gameDetectionStatus: oneOf(
        onboarding.gameDetectionStatus,
        ["running", "notRunning", "probeFailed"] as const,
        "onboarding.gameDetectionStatus",
      ),
      passthroughHotkeyLabel: boundedText(
        onboarding.passthroughHotkeyLabel,
        "onboarding.passthroughHotkeyLabel",
        MAIN_DPS_MAX_TEXT_BYTES,
      ),
      passthroughHotkeyReady: boolean(
        onboarding.passthroughHotkeyReady,
        "onboarding.passthroughHotkeyReady",
      ),
    },
    textTruncated: boolean(source.textTruncated, "textTruncated"),
  };
  validateProjectedTextBudget(snapshot);
  return snapshot;
}

function parseUniqueIds<const T extends readonly string[]>(
  value: unknown,
  allowed: T,
  field: string,
): T[number][] {
  const ids = list(value, field).map((item, index) =>
    oneOf(item, allowed, `${field}[${index}]`),
  );
  if (ids.length > allowed.length || new Set(ids).size !== ids.length) {
    throw new TechnicalContractError(
      `${field} must contain unique identifiers`,
    );
  }
  return ids;
}

export function parseMainDpsResetResult(value: unknown): MainDpsResetResult {
  const source = object(value, "main DPS reset result");
  return {
    snapshot: parseMainDpsSnapshot(source.snapshot),
    undoToken: nullableBoundedText(source.undoToken, "undoToken", 256),
  };
}

export function parseMainDpsEvent(value: unknown): MainDpsSnapshot {
  const event = object(value, "main DPS event");
  if (event.event !== "snapshot")
    throw new TechnicalContractError("Unknown main DPS event");
  return parseMainDpsSnapshot(event.payload);
}

export function parseMainDpsCommandError(value: unknown): MainDpsCommandError {
  return parseTechnicalCommandError(value);
}

function parseRound(value: unknown, index: number): MainDpsRound {
  const row = object(value, `rounds[${index}]`);
  return {
    id: nullableBoundedText(
      row.id,
      `rounds[${index}].id`,
      MAIN_DPS_MAX_TEXT_BYTES,
    ),
    live: boolean(row.live, `rounds[${index}].live`),
    displayTime: nullableBoundedText(
      row.displayTime,
      `rounds[${index}].displayTime`,
      MAIN_DPS_MAX_TEXT_BYTES,
    ),
    abyssFloor: nullableInteger(row.abyssFloor, `rounds[${index}].abyssFloor`),
  };
}

function parseRounds(value: unknown): MainDpsRound[] {
  const rows = list(value, "rounds");
  if (rows.length > MAIN_DPS_MAX_ROUNDS) {
    throw new TechnicalContractError("rounds exceeds the contract limit");
  }
  const rounds = rows.map(parseRound);
  const liveRows = rounds.filter((round) => round.live);
  if (liveRows.length !== 1) {
    throw new TechnicalContractError(
      "rounds must contain exactly one live row",
    );
  }
  for (const [index, round] of rounds.entries()) {
    if (round.live && round.id !== null) {
      throw new TechnicalContractError(
        `rounds[${index}] live row must have a null id`,
      );
    }
    if (!round.live && round.id === null) {
      throw new TechnicalContractError(
        `rounds[${index}] history row must have an id`,
      );
    }
  }
  return rounds;
}

function parseCharacter(value: unknown, index: number): MainDpsCharacter {
  const row = object(value, `characters[${index}]`);
  return {
    characterId: integer(row.characterId, "characterId"),
    name: boundedText(row.name, "name", MAIN_DPS_MAX_TEXT_BYTES),
    hits: boundedText(row.hits, "hits", 32),
    damage: finite(row.damage, "damage"),
    dps: finite(row.dps, "dps"),
    damageSharePercent: finite(row.damageSharePercent, "damageSharePercent"),
    damageTaken: finite(row.damageTaken, "damageTaken"),
    durationSeconds: finite(row.durationSeconds, "durationSeconds"),
    color: nullableBoundedText(row.color, "color", MAIN_DPS_MAX_TEXT_BYTES),
    attribute: nullableBoundedText(
      row.attribute,
      "attribute",
      MAIN_DPS_MAX_TEXT_BYTES,
    ),
  };
}

function validateProjectedTextBudget(snapshot: MainDpsSnapshot): void {
  const encoder = new TextEncoder();
  let bytes = 0;
  const add = (value: string | null): void => {
    if (value !== null) bytes += encoder.encode(value).byteLength;
  };
  add(snapshot.selectedRoundId);
  for (const round of snapshot.rounds) {
    add(round.id);
    add(round.displayTime);
  }
  for (const character of snapshot.readout.characters) {
    add(character.name);
    add(character.hits);
    add(character.color);
    add(character.attribute);
  }
  if (bytes > MAIN_DPS_MAX_PROJECTED_TEXT_BYTES)
    throw new TechnicalContractError(
      `main DPS text exceeds ${MAIN_DPS_MAX_PROJECTED_TEXT_BYTES} UTF-8 bytes`,
    );
}
