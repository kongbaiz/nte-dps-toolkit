import { invoke } from "@tauri-apps/api/core";

import {
  ModStudioContractError,
  parseModStudioCommandError,
  parseModStudioDocument,
  parseModStudioWorkspace,
  type ModStudioDocumentSnapshot,
  type ModStudioWorkspaceSnapshot,
} from "@/lib/tauri/mod-studio-contract";

const COMMANDS = {
  getDocument: "get_mod_studio_document",
  getWorkspace: "get_mod_studio_workspace",
} as const;

interface ModStudioTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
}

export interface ModStudioClient {
  getDocument(id: string): Promise<ModStudioDocumentSnapshot>;
  getWorkspace(): Promise<ModStudioWorkspaceSnapshot>;
}

const tauriTransport: ModStudioTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
};

export function createModStudioClient(
  transport: ModStudioTransport = tauriTransport,
): ModStudioClient {
  async function request<T>(
    command: string,
    parse: (value: unknown) => T,
    arguments_?: Record<string, unknown>,
  ): Promise<T> {
    try {
      return parse(await transport.invoke(command, arguments_));
    } catch (error) {
      if (error instanceof ModStudioContractError) {
        throw error;
      }
      throw parseModStudioCommandError(error);
    }
  }

  return {
    getDocument: (id) =>
      request(COMMANDS.getDocument, parseModStudioDocument, { id }),
    getWorkspace: () => request(COMMANDS.getWorkspace, parseModStudioWorkspace),
  };
}

export const modStudioClient = createModStudioClient();
