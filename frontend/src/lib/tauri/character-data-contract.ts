import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const CHARACTER_DATA_CONTRACT_VERSION = 1;
export const CHARACTER_DATA_MAX_RECORDS = 2_048;

const {
  array: list,
  boolean: flag,
  boundedString: boundedText,
  cssHex6OrEmpty: color,
  decimalString: decimal,
  integer,
  positiveInteger,
  record: object,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

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
          { allowEmpty: true },
        ),
        nameEn: boundedText(
          row.nameEn,
          `characterData.records[${index}].nameEn`,
          128,
          { allowEmpty: true },
        ),
        codename: boundedText(
          row.codename,
          `characterData.records[${index}].codename`,
          128,
          { allowEmpty: true },
        ),
        attribute: boundedText(
          row.attribute,
          `characterData.records[${index}].attribute`,
          16,
          { allowEmpty: true },
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
          { allowEmpty: true },
        ),
      };
    }),
  };
}

export function characterDataError(error: unknown): CharacterDataCommandError {
  return parseTechnicalCommandError(error);
}
