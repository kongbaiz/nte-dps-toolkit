import { invoke } from "@tauri-apps/api/core";

import {
  encryptedIniError,
  parseEncryptedIniSnapshot,
  parseOpenEncryptedIniResult,
  parseSaveEncryptedIniResult,
  type EncryptedIniCommandError,
  type EncryptedIniSnapshot,
  type OpenEncryptedIniResult,
  type SaveEncryptedIniInput,
  type SaveEncryptedIniResult,
} from "@/lib/tauri/encrypted-ini-contract";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";

const COMMANDS = {
  snapshot: "get_encrypted_ini_snapshot",
  open: "open_encrypted_ini",
  reload: "reload_encrypted_ini",
  save: "save_encrypted_ini",
  clear: "clear_encrypted_ini",
} as const;

interface EncryptedIniTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
}

export interface EncryptedIniClient {
  getSnapshot(): Promise<EncryptedIniSnapshot>;
  open(): Promise<OpenEncryptedIniResult>;
  reload(): Promise<EncryptedIniSnapshot>;
  save(input: SaveEncryptedIniInput): Promise<SaveEncryptedIniResult>;
  clear(): Promise<EncryptedIniSnapshot>;
}

const tauriTransport: EncryptedIniTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
};

export function createEncryptedIniClient(
  transport: EncryptedIniTransport = tauriTransport,
): EncryptedIniClient {
  const call = async <T>(operation: () => Promise<T>): Promise<T> => {
    try {
      return await operation();
    } catch (error) {
      if (error instanceof TechnicalContractError) throw error;
      throw encryptedIniError(error);
    }
  };
  return {
    getSnapshot: () =>
      call(async () =>
        parseEncryptedIniSnapshot(await transport.invoke(COMMANDS.snapshot)),
      ),
    open: () =>
      call(async () =>
        parseOpenEncryptedIniResult(await transport.invoke(COMMANDS.open)),
      ),
    reload: () =>
      call(async () =>
        parseEncryptedIniSnapshot(await transport.invoke(COMMANDS.reload)),
      ),
    save: (request) =>
      call(async () =>
        parseSaveEncryptedIniResult(
          await transport.invoke(COMMANDS.save, { request }),
        ),
      ),
    clear: () =>
      call(async () =>
        parseEncryptedIniSnapshot(await transport.invoke(COMMANDS.clear)),
      ),
  };
}

export const encryptedIniClient = createEncryptedIniClient();
export type { EncryptedIniCommandError };
