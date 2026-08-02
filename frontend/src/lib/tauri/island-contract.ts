import {
  TechnicalContractError,
  parseTechnicalCommandError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const ISLAND_CONTRACT_VERSION = 1;

export interface IslandSnapshot {
  contractVersion: number;
  enabled: boolean;
  notice: IslandNotice | null;
}

export interface IslandNotice {
  id: string;
  tone: "info" | "success" | "warning" | "error";
  messageKey: string;
  messageArguments: string[];
  undoAvailable: boolean;
  remainingMs: number;
}

export function parseIslandSnapshot(value: unknown): IslandSnapshot {
  const source = object(value, "island snapshot");
  const version = integer(source.contractVersion, "contractVersion");
  if (version !== ISLAND_CONTRACT_VERSION)
    throw new TechnicalContractError(`Unsupported island contract: ${version}`);
  return {
    contractVersion: version,
    enabled: boolean(source.enabled, "enabled"),
    notice:
      source.notice === null
        ? null
        : parseNotice(object(source.notice, "notice")),
  };
}

export const parseIslandCommandError = (
  value: unknown,
): TechnicalCommandError => parseTechnicalCommandError(value);

function parseNotice(source: Record<string, unknown>): IslandNotice {
  return {
    id: text(source.id, "notice.id"),
    tone: oneOf(
      source.tone,
      ["info", "success", "warning", "error"],
      "notice.tone",
    ),
    messageKey: text(source.messageKey, "notice.messageKey"),
    messageArguments: list(
      source.messageArguments,
      "notice.messageArguments",
    ).map((value, index) => text(value, `notice.messageArguments[${index}]`)),
    undoAvailable: boolean(source.undoAvailable, "notice.undoAvailable"),
    remainingMs: integer(source.remainingMs, "notice.remainingMs"),
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
function boolean(value: unknown, field: string): boolean {
  if (typeof value !== "boolean")
    throw new TechnicalContractError(`${field} must be boolean`);
  return value;
}
function integer(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0)
    throw new TechnicalContractError(`${field} must be a non-negative integer`);
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
