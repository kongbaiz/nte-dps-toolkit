import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const CHARACTER_DATA_CONTRACT_VERSION = 1;
export const CHARACTER_DATA_MAX_RECORDS = 2_048;

export type CharacterDataCommandError = TechnicalCommandError;

export interface CharacterDataRecord {
  id: number;
  nameZh: string;
  nameEn: string;
  codename: string;
  attribute: string;
  verified: boolean;
  color: string;
  avatar: string;
}

export interface CharacterDataRecordInput {
  originalId: string | null;
  id: string;
  nameZh: string;
  nameEn: string;
  codename: string;
  attribute: string;
  verified: boolean;
  color: string;
  avatar: string;
}

export interface CharacterDataSnapshot {
  contractVersion: number;
  generation: string;
  attributes: string[];
  records: CharacterDataRecord[];
}

export function parseCharacterDataSnapshot(
  value: unknown,
): CharacterDataSnapshot {
  const item = object(value, "character data snapshot");
  const contractVersion = integer(
    item.contractVersion,
    "characterData.contractVersion",
  );
  if (contractVersion !== CHARACTER_DATA_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported character data contract version: ${contractVersion}`,
    );
  }
  const records = list(item.records, "characterData.records");
  if (records.length > CHARACTER_DATA_MAX_RECORDS) {
    throw new TechnicalContractError(
      "characterData.records exceeds display bounds",
    );
  }
  return {
    contractVersion,
    generation: decimal(item.generation, "characterData.generation"),
    attributes: list(item.attributes, "characterData.attributes").map(
      (value, index) =>
        boundedText(value, `characterData.attributes[${index}]`, 16),
    ),
    records: records.map((value, index) => {
      const row = object(value, `characterData.records[${index}]`);
      return {
        id: positiveInteger(row.id, `characterData.records[${index}].id`),
        nameZh: boundedText(
          row.nameZh,
          `characterData.records[${index}].nameZh`,
          128,
          true,
        ),
        nameEn: boundedText(
          row.nameEn,
          `characterData.records[${index}].nameEn`,
          128,
          true,
        ),
        codename: boundedText(
          row.codename,
          `characterData.records[${index}].codename`,
          128,
          true,
        ),
        attribute: boundedText(
          row.attribute,
          `characterData.records[${index}].attribute`,
          16,
          true,
        ),
        verified: flag(
          row.verified,
          `characterData.records[${index}].verified`,
        ),
        color: color(row.color, `characterData.records[${index}].color`),
        avatar: boundedText(
          row.avatar,
          `characterData.records[${index}].avatar`,
          512,
          true,
        ),
      };
    }),
  };
}

export function characterDataError(error: unknown): CharacterDataCommandError {
  return parseTechnicalCommandError(error);
}

function object(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TechnicalContractError(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}

function list(value: unknown, field: string): unknown[] {
  if (!Array.isArray(value)) {
    throw new TechnicalContractError(`${field} must be an array`);
  }
  return value;
}

function boundedText(
  value: unknown,
  field: string,
  maxLength: number,
  allowEmpty = false,
): string {
  if (typeof value !== "string") {
    throw new TechnicalContractError(`${field} must be a string`);
  }
  if ((!allowEmpty && value.length === 0) || value.length > maxLength) {
    throw new TechnicalContractError(`${field} has an invalid length`);
  }
  return value;
}

function flag(value: unknown, field: string): boolean {
  if (typeof value !== "boolean") {
    throw new TechnicalContractError(`${field} must be a boolean`);
  }
  return value;
}

function integer(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value)) {
    throw new TechnicalContractError(`${field} must be a safe integer`);
  }
  return value;
}

function positiveInteger(value: unknown, field: string): number {
  const parsed = integer(value, field);
  if (parsed <= 0) {
    throw new TechnicalContractError(`${field} must be positive`);
  }
  return parsed;
}

function decimal(value: unknown, field: string): string {
  if (typeof value !== "string" || !/^(0|[1-9]\d*)$/.test(value)) {
    throw new TechnicalContractError(`${field} must be a decimal string`);
  }
  return value;
}

function color(value: unknown, field: string): string {
  const parsed = boundedText(value, field, 7, true);
  if (parsed !== "" && !/^#[0-9a-fA-F]{6}$/.test(parsed)) {
    throw new TechnicalContractError(`${field} must be #RRGGBB or empty`);
  }
  return parsed;
}
