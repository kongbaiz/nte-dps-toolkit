import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import {
  TechnicalContractError,
  parseTechnicalCommandError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const ISLAND_CONTRACT_VERSION = 1;

const {
  array: list,
  boolean,
  enumValue: oneOf,
  nonNegativeInteger: integer,
  record: object,
  string: text,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

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
