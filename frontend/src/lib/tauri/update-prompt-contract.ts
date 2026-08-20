import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import { parseUpdateSettings, type UpdateSettings } from "./settings-contract";
import { TechnicalContractError } from "./technical-contract";

export const UPDATE_PROMPT_CONTRACT_VERSION = 1;

const { integer, record } = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

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
