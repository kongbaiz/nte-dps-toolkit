import {
  characterDataError,
  parseCharacterDataSnapshot,
  type CharacterDataCommandError,
  type CharacterDataSnapshot,
} from "@/lib/tauri/character-data-contract";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";
import {
  tauriInvokeTransport,
  type InvokeTransport,
} from "@/lib/tauri/stream-client";

const COMMANDS = {
  getSnapshot: "get_character_data_snapshot",
} as const;

export interface CharacterDataClient {
  getSnapshot(): Promise<CharacterDataSnapshot>;
}

export function createCharacterDataClient(
  transport: InvokeTransport = tauriInvokeTransport,
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
  };
}

export const characterDataClient = createCharacterDataClient();
export type { CharacterDataCommandError };
