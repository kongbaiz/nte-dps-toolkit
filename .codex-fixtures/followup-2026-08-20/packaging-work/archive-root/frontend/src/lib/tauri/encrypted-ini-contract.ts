import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const ENCRYPTED_INI_CONTRACT_VERSION = 1;
export const ENCRYPTED_INI_KEYS = ["global", "china"] as const;

const {
  boolean: flag,
  decimalString: decimal,
  integer,
  nonNegativeInteger,
  nullableBoundedString: optionalText,
  positiveInteger,
  record: object,
  string: text,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

export type EncryptedIniKey = (typeof ENCRYPTED_INI_KEYS)[number];
export type EncryptedIniCommandError = TechnicalCommandError;

export interface EncryptedIniSnapshot {
  contractVersion: number;
  generation: string;
  opened: boolean;
  displayPath: string | null;
  fileName: string | null;
  key: EncryptedIniKey;
  plaintext: string;
  encryptedLineCount: number;
  maxBytes: number;
}

export interface OpenEncryptedIniResult {
  opened: boolean;
  snapshot: EncryptedIniSnapshot;
}

export interface SaveEncryptedIniInput {
  expectedGeneration: string;
  key: EncryptedIniKey;
  plaintext: string;
}

export interface SaveEncryptedIniResult {
  saved: boolean;
  snapshot: EncryptedIniSnapshot;
}

export function parseEncryptedIniSnapshot(
  value: unknown,
): EncryptedIniSnapshot {
  const item = object(value, "encryptedIni");
  const contractVersion = integer(
    item.contractVersion,
    "encryptedIni.contractVersion",
  );
  if (contractVersion !== ENCRYPTED_INI_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported encrypted INI contract version: ${contractVersion}`,
    );
  }
  const opened = flag(item.opened, "encryptedIni.opened");
  const displayPath = optionalText(
    item.displayPath,
    "encryptedIni.displayPath",
    32_768,
  );
  const fileName = optionalText(item.fileName, "encryptedIni.fileName", 512);
  if (opened !== (displayPath !== null && fileName !== null)) {
    throw new TechnicalContractError("encryptedIni open state is inconsistent");
  }
  const maxBytes = positiveInteger(item.maxBytes, "encryptedIni.maxBytes");
  const plaintext = text(item.plaintext, "encryptedIni.plaintext");
  if (new TextEncoder().encode(plaintext).length > maxBytes) {
    throw new TechnicalContractError("encryptedIni.plaintext exceeds maxBytes");
  }
  return {
    contractVersion,
    generation: decimal(item.generation, "encryptedIni.generation"),
    opened,
    displayPath,
    fileName,
    key: encryptedIniKey(item.key),
    plaintext,
    encryptedLineCount: nonNegativeInteger(
      item.encryptedLineCount,
      "encryptedIni.encryptedLineCount",
    ),
    maxBytes,
  };
}

export function parseOpenEncryptedIniResult(
  value: unknown,
): OpenEncryptedIniResult {
  const item = object(value, "openEncryptedIniResult");
  return {
    opened: flag(item.opened, "openEncryptedIniResult.opened"),
    snapshot: parseEncryptedIniSnapshot(item.snapshot),
  };
}

export function parseSaveEncryptedIniResult(
  value: unknown,
): SaveEncryptedIniResult {
  const item = object(value, "saveEncryptedIniResult");
  return {
    saved: flag(item.saved, "saveEncryptedIniResult.saved"),
    snapshot: parseEncryptedIniSnapshot(item.snapshot),
  };
}

export function encryptedIniError(error: unknown): EncryptedIniCommandError {
  return parseTechnicalCommandError(error);
}

function encryptedIniKey(value: unknown): EncryptedIniKey {
  if (
    typeof value === "string" &&
    ENCRYPTED_INI_KEYS.includes(value as EncryptedIniKey)
  ) {
    return value as EncryptedIniKey;
  }
  throw new TechnicalContractError("encryptedIni.key is unsupported");
}
