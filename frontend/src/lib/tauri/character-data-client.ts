import { invoke } from "@tauri-apps/api/core";

import {
  characterDataError,
  parseCharacterDataSnapshot,
  type CharacterDataCommandError,
  type CharacterDataRecordInput,
  type CharacterDataSnapshot,
} from "@/lib/tauri/character-data-contract";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";

const COMMANDS = {
  getSnapshot: "get_character_data_snapshot",
  saveRecord: "save_character_data_record",
} as const;

interface CharacterDataTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
}

export interface CharacterDataClient {
  getSnapshot(): Promise<CharacterDataSnapshot>;
  saveRecord(input: CharacterDataRecordInput): Promise<CharacterDataSnapshot>;
}

const tauriTransport: CharacterDataTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
};

export function createCharacterDataClient(
  transport: CharacterDataTransport = tauriTransport,
): CharacterDataClient {
  const request = async (
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<CharacterDataSnapshot> => {
    try {
      return parseCharacterDataSnapshot(
        await transport.invoke(command, arguments_),
      );
    } catch (error) {
      if (error instanceof TechnicalContractError) throw error;
      throw characterDataError(error);
    }
  };
  return {
    getSnapshot: () => request(COMMANDS.getSnapshot),
    saveRecord: (input) => request(COMMANDS.saveRecord, { input }),
  };
}

export const characterDataClient = createCharacterDataClient();
export type { CharacterDataCommandError };
