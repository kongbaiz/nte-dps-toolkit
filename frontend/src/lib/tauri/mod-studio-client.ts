import { Channel, invoke } from "@tauri-apps/api/core";

import {
  ModStudioContractError,
  parseModMarketCatalog,
  parseModStudioCommandError,
  parseModStudioDeployment,
  parseModStudioDirectorySelection,
  parseModStudioDocument,
  parseModStudioGameDirectory,
  parseModStudioRuntimeEvent,
  parseModStudioSdkSchema,
  parseModStudioSubscriptionReceipt,
  parseModStudioWorkspace,
  type ModStudioCommandError,
  type ModMarketCatalogSnapshot,
  type ModStudioDocumentSnapshot,
  type ModStudioDeploymentSnapshot,
  type ModStudioDirectorySelectionSnapshot,
  type ModStudioGameDirectorySnapshot,
  type ModStudioGameRegion,
  type ModStudioRuntimeEvent,
  type ModStudioSdkSchemaSnapshot,
  type ModStudioWorkspaceSnapshot,
} from "@/lib/tauri/mod-studio-contract";

const COMMANDS = {
  getMarketCatalog: "get_mod_market_catalog",
  installMarketItem: "install_mod_market_item",
  deleteDocument: "delete_mod_studio_document",
  chooseGameDirectory: "choose_mod_studio_game_directory",
  createDocument: "create_mod_studio_document",
  getDeployment: "get_mod_studio_deployment",
  getGameDirectory: "get_mod_studio_game_directory",
  getDocument: "get_mod_studio_document",
  getSdkSchema: "get_mod_studio_sdk_schema",
  getWorkspace: "get_mod_studio_workspace",
  openFolder: "open_mod_studio_folder",
  saveDocument: "save_mod_studio_document",
  setEnabled: "set_mod_studio_document_enabled",
  setGameDirectory: "set_mod_studio_game_directory",
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
  getMarketCatalog(): Promise<ModMarketCatalogSnapshot>;
  installMarketItem(id: string): Promise<ModStudioDocumentSnapshot>;
  deleteDocument(id: string): Promise<ModStudioWorkspaceSnapshot>;
  chooseGameDirectory(
    region: ModStudioGameRegion,
  ): Promise<ModStudioDirectorySelectionSnapshot>;
  createDocument(id: string): Promise<ModStudioDocumentSnapshot>;
  getDeployment(
    region: ModStudioGameRegion | null,
    gameDirectory: string | null,
  ): Promise<ModStudioDeploymentSnapshot>;
  getGameDirectory(
    region: ModStudioGameRegion,
  ): Promise<ModStudioGameDirectorySnapshot>;
  getDocument(id: string): Promise<ModStudioDocumentSnapshot>;
  getSdkSchema(): Promise<ModStudioSdkSchemaSnapshot>;
  getWorkspace(): Promise<ModStudioWorkspaceSnapshot>;
  openFolder(): Promise<true>;
  saveDocument(id: string, source: string): Promise<ModStudioDocumentSnapshot>;
  setEnabled(id: string, enabled: boolean): Promise<ModStudioWorkspaceSnapshot>;
  setGameDirectory(
    region: ModStudioGameRegion,
    gameDirectory: string | null,
  ): Promise<ModStudioGameDirectorySnapshot>;
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
    getMarketCatalog: () =>
      request(COMMANDS.getMarketCatalog, parseModMarketCatalog),
    installMarketItem: (id) =>
      request(COMMANDS.installMarketItem, parseModStudioDocument, { id }),
    deleteDocument: (id) =>
      request(COMMANDS.deleteDocument, parseModStudioWorkspace, { id }),
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
    getGameDirectory: (region) =>
      request(COMMANDS.getGameDirectory, parseModStudioGameDirectory, {
        region,
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
    setGameDirectory: (region, gameDirectory) =>
      request(COMMANDS.setGameDirectory, parseModStudioGameDirectory, {
        region,
        gameDirectory,
      }),
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
