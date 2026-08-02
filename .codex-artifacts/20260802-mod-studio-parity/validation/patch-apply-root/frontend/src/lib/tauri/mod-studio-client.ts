import { Channel, invoke } from "@tauri-apps/api/core";

import {
  ModStudioContractError,
  parseModStudioCommandError,
  parseModStudioDeployment,
  parseModStudioDirectorySelection,
  parseModStudioDocument,
  parseModStudioRuntimeEvent,
  parseModStudioSdkSchema,
  parseModStudioSubscriptionReceipt,
  parseModStudioWorkspace,
  type ModStudioCommandError,
  type ModStudioDocumentSnapshot,
  type ModStudioDeploymentSnapshot,
  type ModStudioDirectorySelectionSnapshot,
  type ModStudioGameRegion,
  type ModStudioRuntimeEvent,
  type ModStudioSdkSchemaSnapshot,
  type ModStudioWorkspaceSnapshot,
} from "@/lib/tauri/mod-studio-contract";

const COMMANDS = {
  chooseGameDirectory: "choose_mod_studio_game_directory",
  createDocument: "create_mod_studio_document",
  getDeployment: "get_mod_studio_deployment",
  getDocument: "get_mod_studio_document",
  getSdkSchema: "get_mod_studio_sdk_schema",
  getWorkspace: "get_mod_studio_workspace",
  openFolder: "open_mod_studio_folder",
  saveDocument: "save_mod_studio_document",
  setEnabled: "set_mod_studio_document_enabled",
  setLoaderEnabled: "set_mod_studio_loader_enabled",
  subscribeRuntime: "subscribe_mod_studio_runtime",
  unsubscribeRuntime: "unsubscribe_mod_studio_runtime",
} as const;

interface ModStudioTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  createChannel(onMessage: (message: unknown) => void): unknown;
}

export interface ModStudioClient {
  chooseGameDirectory(
    region: ModStudioGameRegion,
  ): Promise<ModStudioDirectorySelectionSnapshot>;
  createDocument(id: string): Promise<ModStudioDocumentSnapshot>;
  getDeployment(
    region: ModStudioGameRegion | null,
    gameDirectory: string | null,
  ): Promise<ModStudioDeploymentSnapshot>;
  getDocument(id: string): Promise<ModStudioDocumentSnapshot>;
  getSdkSchema(): Promise<ModStudioSdkSchemaSnapshot>;
  getWorkspace(): Promise<ModStudioWorkspaceSnapshot>;
  openFolder(): Promise<true>;
  saveDocument(id: string, source: string): Promise<ModStudioDocumentSnapshot>;
  setEnabled(id: string, enabled: boolean): Promise<ModStudioWorkspaceSnapshot>;
  setLoaderEnabled(
    region: ModStudioGameRegion,
    enabled: boolean,
    gameDirectory: string | null,
  ): Promise<ModStudioDeploymentSnapshot>;
  subscribeRuntime(
    onEvent: (event: ModStudioRuntimeEvent) => void,
    onError: (error: ModStudioCommandError) => void,
  ): () => Promise<void>;
}

const tauriTransport: ModStudioTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
  createChannel: (onMessage) => {
    const channel = new Channel<unknown>();
    channel.onmessage = onMessage;
    return channel;
  },
};

export function createModStudioClient(
  transport: ModStudioTransport = tauriTransport,
  createSubscriptionId: () => string = () => crypto.randomUUID(),
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
    chooseGameDirectory: (region) =>
      request(COMMANDS.chooseGameDirectory, parseModStudioDirectorySelection, {
        region,
      }),
    createDocument: (id) =>
      request(COMMANDS.createDocument, parseModStudioDocument, { id }),
    getDeployment: (region, gameDirectory) =>
      request(COMMANDS.getDeployment, parseModStudioDeployment, {
        region,
        gameDirectory,
      }),
    getDocument: (id) =>
      request(COMMANDS.getDocument, parseModStudioDocument, { id }),
    getSdkSchema: () => request(COMMANDS.getSdkSchema, parseModStudioSdkSchema),
    getWorkspace: () => request(COMMANDS.getWorkspace, parseModStudioWorkspace),
    openFolder: () =>
      request(COMMANDS.openFolder, (value) => {
        if (value !== true) {
          throw new ModStudioContractError("open Mod folder result is invalid");
        }
        return true as const;
      }),
    saveDocument: (id, source) =>
      request(COMMANDS.saveDocument, parseModStudioDocument, { id, source }),
    setEnabled: (id, enabled) =>
      request(COMMANDS.setEnabled, parseModStudioWorkspace, { id, enabled }),
    setLoaderEnabled: (region, enabled, gameDirectory) =>
      request(COMMANDS.setLoaderEnabled, parseModStudioDeployment, {
        region,
        enabled,
        gameDirectory,
      }),
    subscribeRuntime: (onEvent, onError) => {
      const subscriptionId = createSubscriptionId();
      let closed = false;
      const onMessage = (message: unknown) => {
        if (closed) {
          return;
        }
        try {
          onEvent(parseModStudioRuntimeEvent(message));
        } catch (error) {
          onError(parseModStudioCommandError(error));
        }
      };
      const onEventChannel = transport.createChannel(onMessage);
      const receipt = transport
        .invoke(COMMANDS.subscribeRuntime, {
          subscriptionId,
          onEvent: onEventChannel,
        })
        .then(parseModStudioSubscriptionReceipt)
        .catch((error: unknown) => {
          if (!closed) {
            onError(parseModStudioCommandError(error));
          }
          return undefined;
        });

      return async () => {
        if (closed) {
          return;
        }
        closed = true;
        const activeReceipt = await receipt;
        if (activeReceipt) {
          await transport.invoke(COMMANDS.unsubscribeRuntime, {
            subscriptionId: activeReceipt.subscriptionId,
          });
        }
      };
    },
  };
}

export const modStudioClient = createModStudioClient();
