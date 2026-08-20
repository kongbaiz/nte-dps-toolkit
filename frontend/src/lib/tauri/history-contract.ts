import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const HISTORY_CONTRACT_VERSION = 2;
export const HISTORY_MAX_RECORDS = 200;
export const HISTORY_MAX_INLINE_EXPORT_CHARACTERS = 8 * 1024 * 1024;

const {
  array: list,
  boundedString,
  boundedMap,
  boolean: flag,
  decimalString: decimal,
  enumValue,
  finiteNumber: number,
  integer,
  nonNegativeInteger,
  nullableEnumValue: nullableEnum,
  nullableNonNegativeInteger: nullableInteger,
  record: object,
  string: text,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

export type HistoryCommandError = TechnicalCommandError;
export type HistoryLine = "upper" | "lower";

export interface HistoryCharacter {
  charId: number;
  name: string;
  hits: string;
  damage: number;
  dps: number;
  damageSharePercent: number;
  hitsTaken: string;
  damageTaken: number;
}

export interface HistorySkill {
  charId: number;
  charName: string;
  name: string;
  category: string;
  hits: string;
  damage: number;
  damageSharePercent: number;
  isFollowUp: boolean;
}

export interface HistoryHalf {
  half: "first" | "second";
  durationSeconds: number;
  totalDamage: number;
  totalDps: number;
  characters: HistoryCharacter[];
  skills: HistorySkill[];
  hiddenCharacterCount: number;
  hiddenSkillCount: number;
}

export interface HistorySummary {
  durationSeconds: number;
  dpsTimeBasis: "subtract_time_stop" | "wall_clock";
  totalDamage: number;
  totalDps: number;
  totalDamageTaken: number;
  totalHits: string;
  reactionDamageSeparated: boolean;
  characters: HistoryCharacter[];
  skills: HistorySkill[];
  abyss: {
    detected: boolean;
    floor: number | null;
    activeHalf: "first" | "second" | null;
    success: boolean;
    firstHalf: HistoryHalf | null;
    secondHalf: HistoryHalf | null;
  };
  quality: {
    source: "live" | "pcapng_replay" | "json_replay" | "unknown";
    packetCount: string;
    hitCount: string;
    unmappedSkillHits: string;
    unknownCharacterHits: string;
  };
  hiddenCharacterCount: number;
  hiddenSkillCount: number;
}

export interface HistoryRecord {
  id: string;
  displayTime: string;
  recordedAt: string;
  hasDetails: boolean;
  partyLabel: string;
  canSetUpperPrediction: boolean;
  canSetLowerPrediction: boolean;
  summary: HistorySummary;
}

export interface HistorySnapshot {
  contractVersion: number;
  revision: string;
  maxImportBytes: string;
  skippedFiles: number;
  records: HistoryRecord[];
}

export interface HistoryDeleteResult {
  history: HistorySnapshot;
  undoToken: string;
  undoExpiresMs: number;
}

export interface HistoryImportFileResult {
  performed: boolean;
  importedRecordId: string | null;
  history: HistorySnapshot;
}

export interface HistoryFileActionResult {
  performed: boolean;
}

export interface HistoryComparison {
  leftId: string;
  rightId: string;
  totalDpsDelta: number;
  totalDamageDelta: number;
  durationDelta: number;
  differentTimeBasis: boolean;
  differentReactionAccounting: boolean;
  characterDeltas: Array<{
    charId: number;
    name: string;
    leftDps: number;
    rightDps: number;
    deltaDps: number;
    leftDamage: number;
    rightDamage: number;
    deltaDamage: number;
  }>;
  skillDeltas: Array<{
    name: string;
    category: string;
    leftDamage: number;
    rightDamage: number;
    deltaDamage: number;
  }>;
}

export interface HistoryExport {
  fileName: string;
  json: string;
}

export function parseHistorySnapshot(value: unknown): HistorySnapshot {
  const item = object(value, "history snapshot");
  const contractVersion = integer(
    item.contractVersion,
    "history.contractVersion",
  );
  if (contractVersion !== HISTORY_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported history contract version: ${contractVersion}`,
    );
  }
  const records = list(item.records, "history.records");
  if (records.length > HISTORY_MAX_RECORDS) {
    throw new TechnicalContractError(
      "history.records exceeds the supported limit",
    );
  }
  return {
    contractVersion,
    revision: decimal(item.revision, "history.revision"),
    maxImportBytes: decimal(item.maxImportBytes, "history.maxImportBytes"),
    skippedFiles: nonNegativeInteger(item.skippedFiles, "history.skippedFiles"),
    records: records.map((record, index) =>
      parseRecord(record, `history.records[${index}]`),
    ),
  };
}

export function parseHistoryEvent(value: unknown): HistorySnapshot {
  const item = object(value, "history event");
  if (item.event !== "snapshot") {
    throw new TechnicalContractError("Unsupported history event");
  }
  return parseHistorySnapshot(item.payload);
}

export function parseHistoryDeleteResult(value: unknown): HistoryDeleteResult {
  const item = object(value, "history delete result");
  return {
    history: parseHistorySnapshot(item.history),
    undoToken: text(item.undoToken, "history delete undoToken"),
    undoExpiresMs: nonNegativeInteger(
      item.undoExpiresMs,
      "history delete undoExpiresMs",
    ),
  };
}

export function parseHistoryImportFileResult(
  value: unknown,
): HistoryImportFileResult {
  const item = object(value, "history import file result");
  return {
    performed: flag(item.performed, "history import file.performed"),
    importedRecordId:
      item.importedRecordId === null
        ? null
        : text(item.importedRecordId, "history import file.importedRecordId"),
    history: parseHistorySnapshot(item.history),
  };
}

export function parseHistoryFileActionResult(
  value: unknown,
): HistoryFileActionResult {
  const item = object(value, "history file action result");
  return {
    performed: flag(item.performed, "history file action.performed"),
  };
}

export function parseHistoryComparison(value: unknown): HistoryComparison {
  const item = object(value, "history comparison");
  const characterDeltas = list(
    item.characterDeltas,
    "comparison.characterDeltas",
  );
  const skillDeltas = list(item.skillDeltas, "comparison.skillDeltas");
  if (characterDeltas.length > 8 || skillDeltas.length > 8) {
    throw new TechnicalContractError(
      "history comparison exceeds display bounds",
    );
  }
  return {
    leftId: text(item.leftId, "comparison.leftId"),
    rightId: text(item.rightId, "comparison.rightId"),
    totalDpsDelta: number(item.totalDpsDelta, "comparison.totalDpsDelta"),
    totalDamageDelta: number(
      item.totalDamageDelta,
      "comparison.totalDamageDelta",
    ),
    durationDelta: number(item.durationDelta, "comparison.durationDelta"),
    differentTimeBasis: flag(
      item.differentTimeBasis,
      "comparison.differentTimeBasis",
    ),
    differentReactionAccounting: flag(
      item.differentReactionAccounting,
      "comparison.differentReactionAccounting",
    ),
    characterDeltas: characterDeltas.map((value, index) => {
      const row = object(value, `comparison.characterDeltas[${index}]`);
      return {
        charId: nonNegativeInteger(
          row.charId,
          `comparison.characterDeltas[${index}].charId`,
        ),
        name: text(row.name, `comparison.characterDeltas[${index}].name`),
        leftDps: number(
          row.leftDps,
          `comparison.characterDeltas[${index}].leftDps`,
        ),
        rightDps: number(
          row.rightDps,
          `comparison.characterDeltas[${index}].rightDps`,
        ),
        deltaDps: number(
          row.deltaDps,
          `comparison.characterDeltas[${index}].deltaDps`,
        ),
        leftDamage: number(
          row.leftDamage,
          `comparison.characterDeltas[${index}].leftDamage`,
        ),
        rightDamage: number(
          row.rightDamage,
          `comparison.characterDeltas[${index}].rightDamage`,
        ),
        deltaDamage: number(
          row.deltaDamage,
          `comparison.characterDeltas[${index}].deltaDamage`,
        ),
      };
    }),
    skillDeltas: skillDeltas.map((value, index) => {
      const row = object(value, `comparison.skillDeltas[${index}]`);
      return {
        name: text(row.name, `comparison.skillDeltas[${index}].name`),
        category: text(
          row.category,
          `comparison.skillDeltas[${index}].category`,
        ),
        leftDamage: number(
          row.leftDamage,
          `comparison.skillDeltas[${index}].leftDamage`,
        ),
        rightDamage: number(
          row.rightDamage,
          `comparison.skillDeltas[${index}].rightDamage`,
        ),
        deltaDamage: number(
          row.deltaDamage,
          `comparison.skillDeltas[${index}].deltaDamage`,
        ),
      };
    }),
  };
}

export function parseHistoryExport(value: unknown): HistoryExport {
  const item = object(value, "history export");
  return {
    fileName: text(item.fileName, "history export.fileName"),
    json: boundedString(
      item.json,
      "history export.json",
      HISTORY_MAX_INLINE_EXPORT_CHARACTERS,
    ),
  };
}

export function historyError(error: unknown): HistoryCommandError {
  return parseTechnicalCommandError(error);
}

function parseRecord(value: unknown, field: string): HistoryRecord {
  const item = object(value, field);
  const summary = object(item.summary, `${field}.summary`);
  const abyss = object(summary.abyss, `${field}.summary.abyss`);
  const quality = object(summary.quality, `${field}.summary.quality`);
  return {
    id: text(item.id, `${field}.id`),
    displayTime: text(item.displayTime, `${field}.displayTime`),
    recordedAt: text(item.recordedAt, `${field}.recordedAt`),
    hasDetails: flag(item.hasDetails, `${field}.hasDetails`),
    partyLabel: text(item.partyLabel, `${field}.partyLabel`),
    canSetUpperPrediction: flag(
      item.canSetUpperPrediction,
      `${field}.canSetUpperPrediction`,
    ),
    canSetLowerPrediction: flag(
      item.canSetLowerPrediction,
      `${field}.canSetLowerPrediction`,
    ),
    summary: {
      durationSeconds: number(
        summary.durationSeconds,
        `${field}.summary.durationSeconds`,
      ),
      dpsTimeBasis: enumValue(
        summary.dpsTimeBasis,
        ["subtract_time_stop", "wall_clock"] as const,
        `${field}.summary.dpsTimeBasis`,
      ),
      totalDamage: number(summary.totalDamage, `${field}.summary.totalDamage`),
      totalDps: number(summary.totalDps, `${field}.summary.totalDps`),
      totalDamageTaken: number(
        summary.totalDamageTaken,
        `${field}.summary.totalDamageTaken`,
      ),
      totalHits: decimal(summary.totalHits, `${field}.summary.totalHits`),
      reactionDamageSeparated: flag(
        summary.reactionDamageSeparated,
        `${field}.summary.reactionDamageSeparated`,
      ),
      characters: boundedMap(
        summary.characters,
        `${field}.summary.characters`,
        8,
        parseCharacter,
      ),
      skills: boundedMap(
        summary.skills,
        `${field}.summary.skills`,
        8,
        parseSkill,
      ),
      abyss: {
        detected: flag(abyss.detected, `${field}.summary.abyss.detected`),
        floor: nullableInteger(abyss.floor, `${field}.summary.abyss.floor`),
        activeHalf: nullableEnum(
          abyss.activeHalf,
          ["first", "second"] as const,
          `${field}.summary.abyss.activeHalf`,
        ),
        success: flag(abyss.success, `${field}.summary.abyss.success`),
        firstHalf:
          abyss.firstHalf === null
            ? null
            : parseHalf(abyss.firstHalf, `${field}.summary.abyss.firstHalf`),
        secondHalf:
          abyss.secondHalf === null
            ? null
            : parseHalf(abyss.secondHalf, `${field}.summary.abyss.secondHalf`),
      },
      quality: {
        source: enumValue(
          quality.source,
          ["live", "pcapng_replay", "json_replay", "unknown"] as const,
          `${field}.summary.quality.source`,
        ),
        packetCount: decimal(
          quality.packetCount,
          `${field}.summary.quality.packetCount`,
        ),
        hitCount: decimal(
          quality.hitCount,
          `${field}.summary.quality.hitCount`,
        ),
        unmappedSkillHits: decimal(
          quality.unmappedSkillHits,
          `${field}.summary.quality.unmappedSkillHits`,
        ),
        unknownCharacterHits: decimal(
          quality.unknownCharacterHits,
          `${field}.summary.quality.unknownCharacterHits`,
        ),
      },
      hiddenCharacterCount: nonNegativeInteger(
        summary.hiddenCharacterCount,
        `${field}.summary.hiddenCharacterCount`,
      ),
      hiddenSkillCount: nonNegativeInteger(
        summary.hiddenSkillCount,
        `${field}.summary.hiddenSkillCount`,
      ),
    },
  };
}

function parseCharacter(value: unknown, field: string): HistoryCharacter {
  const row = object(value, field);
  return {
    charId: nonNegativeInteger(row.charId, `${field}.charId`),
    name: text(row.name, `${field}.name`),
    hits: decimal(row.hits, `${field}.hits`),
    damage: number(row.damage, `${field}.damage`),
    dps: number(row.dps, `${field}.dps`),
    damageSharePercent: number(
      row.damageSharePercent,
      `${field}.damageSharePercent`,
    ),
    hitsTaken: decimal(row.hitsTaken, `${field}.hitsTaken`),
    damageTaken: number(row.damageTaken, `${field}.damageTaken`),
  };
}

function parseSkill(value: unknown, field: string): HistorySkill {
  const row = object(value, field);
  return {
    charId: nonNegativeInteger(row.charId, `${field}.charId`),
    charName: text(row.charName, `${field}.charName`),
    name: text(row.name, `${field}.name`),
    category: text(row.category, `${field}.category`),
    hits: decimal(row.hits, `${field}.hits`),
    damage: number(row.damage, `${field}.damage`),
    damageSharePercent: number(
      row.damageSharePercent,
      `${field}.damageSharePercent`,
    ),
    isFollowUp: flag(row.isFollowUp, `${field}.isFollowUp`),
  };
}

function parseHalf(value: unknown, field: string): HistoryHalf {
  const row = object(value, field);
  return {
    half: enumValue(row.half, ["first", "second"] as const, `${field}.half`),
    durationSeconds: number(row.durationSeconds, `${field}.durationSeconds`),
    totalDamage: number(row.totalDamage, `${field}.totalDamage`),
    totalDps: number(row.totalDps, `${field}.totalDps`),
    characters: boundedMap(
      row.characters,
      `${field}.characters`,
      8,
      parseCharacter,
    ),
    skills: boundedMap(row.skills, `${field}.skills`, 8, parseSkill),
    hiddenCharacterCount: nonNegativeInteger(
      row.hiddenCharacterCount,
      `${field}.hiddenCharacterCount`,
    ),
    hiddenSkillCount: nonNegativeInteger(
      row.hiddenSkillCount,
      `${field}.hiddenSkillCount`,
    ),
  };
}
