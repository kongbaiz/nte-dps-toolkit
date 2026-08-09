import {
  parseCaptureSnapshot,
  parseTechnicalCommandError,
  TechnicalContractError,
  type CaptureSnapshot,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const MAIN_DPS_CONTRACT_VERSION = 4;
export const MAIN_DPS_MAX_HISTORY_RECORDS = 200;
export const MAIN_DPS_MAX_ROUNDS = MAIN_DPS_MAX_HISTORY_RECORDS + 1;

export interface MainDpsSnapshot {
  contractVersion: number;
  generation: string;
  captureGeneration: string;
  presentationGeneration: string;
  historyGeneration: string;
  adapterVersion: string;
  capture: CaptureSnapshot;
  processingPaused: boolean;
  pausedPendingEvents: string;
  pausedDebugPackets: string;
  replayRunning: boolean;
  alwaysOnTop: boolean;
  passthrough: boolean;
  appearance: MainDpsAppearance;
  rounds: MainDpsRound[];
  selectedRoundId: string | null;
  readout: MainDpsReadout;
  actions: MainDpsActions;
  gameDetected: boolean;
  gameDetectionStatus: GameDetectionStatus;
  hasLiveSessionData: boolean;
  onboarding: MainDpsOnboarding;
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

export interface MainDpsOnboarding {
  done: boolean;
  step: number;
  captureDeviceCount: number;
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
  characterDirectDamage: number;
  characterReactionDamage: number;
  sharedDamage: number;
  unattributedDamage: number;
  separateReactionDamage: boolean;
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
  const actions = object(source.actions, "actions");
  const onboarding = object(source.onboarding, "onboarding");
  const dataState = oneOf(
    readout.dataState,
    ["empty", "preview", "live"],
    "readout.dataState",
  );
  const rounds = parseRounds(source.rounds);

  return {
    contractVersion: version,
    generation: text(source.generation, "generation"),
    captureGeneration: text(source.captureGeneration, "captureGeneration"),
    presentationGeneration: text(
      source.presentationGeneration,
      "presentationGeneration",
    ),
    historyGeneration: text(source.historyGeneration, "historyGeneration"),
    adapterVersion: text(source.adapterVersion, "adapterVersion"),
    capture: parseCaptureSnapshot(source.capture),
    processingPaused: boolean(source.processingPaused, "processingPaused"),
    pausedPendingEvents: unsignedIntegerText(
      source.pausedPendingEvents,
      "pausedPendingEvents",
    ),
    pausedDebugPackets: unsignedIntegerText(
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
    rounds,
    selectedRoundId: nullableText(source.selectedRoundId, "selectedRoundId"),
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
      characters: list(readout.characters, "readout.characters")
        .slice(0, 64)
        .map(parseCharacter),
      damageAttribution: {
        totalDamage: finite(
          damageAttribution.totalDamage,
          "damageAttribution.totalDamage",
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
      gameDetected: boolean(onboarding.gameDetected, "onboarding.gameDetected"),
      gameDetectionStatus: oneOf(
        onboarding.gameDetectionStatus,
        ["running", "notRunning", "probeFailed"] as const,
        "onboarding.gameDetectionStatus",
      ),
      passthroughHotkeyLabel: text(
        onboarding.passthroughHotkeyLabel,
        "onboarding.passthroughHotkeyLabel",
      ),
      passthroughHotkeyReady: boolean(
        onboarding.passthroughHotkeyReady,
        "onboarding.passthroughHotkeyReady",
      ),
    },
  };
}

export function parseMainDpsResetResult(value: unknown): MainDpsResetResult {
  const source = object(value, "main DPS reset result");
  return {
    snapshot: parseMainDpsSnapshot(source.snapshot),
    undoToken: nullableText(source.undoToken, "undoToken"),
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
    id: nullableText(row.id, `rounds[${index}].id`),
    live: boolean(row.live, `rounds[${index}].live`),
    displayTime: nullableText(row.displayTime, `rounds[${index}].displayTime`),
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
    name: text(row.name, "name"),
    hits: text(row.hits, "hits"),
    damage: finite(row.damage, "damage"),
    dps: finite(row.dps, "dps"),
    damageSharePercent: finite(row.damageSharePercent, "damageSharePercent"),
    damageTaken: finite(row.damageTaken, "damageTaken"),
    durationSeconds: finite(row.durationSeconds, "durationSeconds"),
    color: nullableText(row.color, "color"),
    attribute: nullableText(row.attribute, "attribute"),
  };
}

function object(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new TechnicalContractError(`${field} must be an object`);
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
function nullableText(value: unknown, field: string): string | null {
  return value === null ? null : text(value, field);
}
function finite(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value))
    throw new TechnicalContractError(`${field} must be finite`);
  return value;
}
function integer(value: unknown, field: string): number {
  const number = finite(value, field);
  if (!Number.isInteger(number))
    throw new TechnicalContractError(`${field} must be an integer`);
  return number;
}
function nullableInteger(value: unknown, field: string): number | null {
  return value === null ? null : integer(value, field);
}
function unsignedIntegerText(value: unknown, field: string): string {
  const candidate = text(value, field);
  if (!/^\d+$/.test(candidate))
    throw new TechnicalContractError(
      `${field} must be an unsigned integer string`,
    );
  return candidate;
}
function boolean(value: unknown, field: string): boolean {
  if (typeof value !== "boolean")
    throw new TechnicalContractError(`${field} must be boolean`);
  return value;
}
function oneOf<const T extends readonly string[]>(
  value: unknown,
  values: T,
  field: string,
): T[number] {
  const candidate = text(value, field);
  if (!values.includes(candidate))
    throw new TechnicalContractError(`${field} is unsupported`);
  return candidate as T[number];
}
