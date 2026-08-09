import { parseUpdateSettings, type UpdateSettings } from "./settings-contract";
import { TechnicalContractError } from "./technical-contract";

export const UPDATE_PROMPT_CONTRACT_VERSION = 1;

export interface UpdatePromptSnapshot {
  contractVersion: number;
  updates: UpdateSettings;
}

export function parseUpdatePromptSnapshot(
  value: unknown,
): UpdatePromptSnapshot {
  const source = record(value, "update prompt");
  const contractVersion = integer(
    source.contractVersion,
    "updatePrompt.contractVersion",
  );
  if (contractVersion !== UPDATE_PROMPT_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported update prompt contract: ${contractVersion}`,
    );
  }
  return {
    contractVersion,
    updates: parseUpdateSettings(
      record(source.updates, "updatePrompt.updates"),
    ),
  };
}

function record(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TechnicalContractError(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}

function integer(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isInteger(value)) {
    throw new TechnicalContractError(`${field} must be an integer`);
  }
  return value;
}
