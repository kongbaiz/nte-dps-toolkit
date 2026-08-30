import {
  ModStudioContractError,
  parseModMarketCatalog,
  parseModLoaderRuntime,
  parseModStudioCommandError,
  parseModStudioDeployment,
  parseModStudioDirectorySelection,
  parseModStudioDocument,
  parseModStudioGameDirectory,
  parseModStudioLoadingMethodPreference,
  parseModStudioRuntimeEvent,
  parseModStudioSdkSchema,
  parseModStudioWorkspace,
  type ModStudioCommandError,
  type ModMarketCatalogSnapshot,
  type ModLoaderRuntimeSnapshot,
  type ModStudioDocumentSnapshot,
  type ModStudioDeploymentSnapshot,
  type ModStudioDirectorySelectionSnapshot,
  type ModStudioGameDirectorySnapshot,
  type ModStudioGameRegion,
  type ModLoadingMethod,
  type ModStudioLoadingMethodPreferenceSnapshot,
  type ModStudioRuntimeEvent,
  type ModStudioSdkSchemaSnapshot,
  type ModStudioWorkspaceSnapshot,
} from "@/lib/tauri/mod-studio-contract";
import {
  subscribeStream,
  tauriStreamTransport,
  type StreamTransport,
} from "@/lib/tauri/stream-client";

const COMMANDS = {
  acknowledgeRisk: "acknowledge_mod_studio_risk",
  getMarketCatalog: "get_mod_market_catalog",
  getLoaderRuntime: "get_mod_loader_runtime",
  getLoaderGameRunning: "get_mod_loader_game_running",
  installMarketItem: "install_mod_market_item",
  deleteDocument: "delete_mod_studio_document",
  chooseGameDirectory: "choose_mod_studio_game_directory",
  createDocument: "create_mod_studio_document",
  getDeployment: "get_mod_studio_deployment",
  getGameDirectory: "get_mod_studio_game_directory",
  getLoadingMethod: "get_mod_studio_loading_method",
  getDocument: "get_mod_studio_document",
  getSdkSchema: "get_mod_studio_sdk_schema",
  getWorkspace: "get_mod_studio_workspace",
  openFolder: "open_mod_studio_folder",
  openLoaderDirectory: "open_mod_loader_directory",
  saveDocument: "save_mod_studio_document",
  setEnabled: "set_mod_studio_document_enabled",
  setGameDirectory: "set_mod_studio_game_directory",
  setLoadingMethod: "set_mod_studio_loading_method",
  setLoaderEnabled: "set_mod_studio_loader_enabled",
  setLoaderRunning: "set_mod_loader_running",
  subscribeRuntime: "subscribe_mod_studio_runtime",
  unsubscribeRuntime: "unsubscribe_mod_studio_runtime",
} as const;

export interface ModStudioClient {
  acknowledgeRisk(): Promise<ModStudioLoadingMethodPreferenceSnapshot>;
  getLoaderRuntime(): Promise<ModLoaderRuntimeSnapshot>;
  getLoaderGameRunning(): Promise<boolean>;
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
  getLoadingMethod(): Promise<ModStudioLoadingMethodPreferenceSnapshot>;
  getDocument(id: string): Promise<ModStudioDocumentSnapshot>;
  getSdkSchema(): Promise<ModStudioSdkSchemaSnapshot>;
  getWorkspace(): Promise<ModStudioWorkspaceSnapshot>;
  openFolder(): Promise<true>;
  openLoaderDirectory(): Promise<true>;
  saveDocument(id: string, source: string): Promise<ModStudioDocumentSnapshot>;
  setEnabled(id: string, enabled: boolean): Promise<ModStudioWorkspaceSnapshot>;
  setGameDirectory(
    region: ModStudioGameRegion,
    gameDirectory: string | null,
  ): Promise<ModStudioGameDirectorySnapshot>;
  setLoadingMethod(
    method: ModLoadingMethod,
  ): Promise<ModStudioLoadingMethodPreferenceSnapshot>;
  setLoaderEnabled(
    region: ModStudioGameRegion,
    enabled: boolean,
    gameDirectory: string | null,
  ): Promise<ModStudioDeploymentSnapshot>;
  setLoaderRunning(
    running: boolean,
    terminateProcesses: boolean,
  ): Promise<ModLoaderRuntimeSnapshot>;
  subscribeRuntime(
    onEvent: (event: ModStudioRuntimeEvent) => void,
    onError: (error: ModStudioCommandError) => void,
  ): () => Promise<void>;
}

export function createModStudioClient(
  transport: StreamTransport = tauriStreamTransport,
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
    acknowledgeRisk: () =>
      request(COMMANDS.acknowledgeRisk, parseModStudioLoadingMethodPreference),
    getLoaderRuntime: () =>
      request(COMMANDS.getLoaderRuntime, parseModLoaderRuntime),
    getLoaderGameRunning: () =>
      request(COMMANDS.getLoaderGameRunning, (value) => {
        if (typeof value !== "boolean") {
          throw new TypeError("Mod Loader game process state is invalid");
        }
        return value;
      }),
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
    getLoadingMethod: () =>
      request(COMMANDS.getLoadingMethod, parseModStudioLoadingMethodPreference),
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
    openLoaderDirectory: () =>
      request(COMMANDS.openLoaderDirectory, (value) => {
        if (value !== true) {
          throw new ModStudioContractError(
            "open Mod Loader directory result is invalid",
          );
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
    setLoadingMethod: (method) =>
      request(
        COMMANDS.setLoadingMethod,
        parseModStudioLoadingMethodPreference,
        { method },
      ),
    setLoaderEnabled: (region, enabled, gameDirectory) =>
      request(COMMANDS.setLoaderEnabled, parseModStudioDeployment, {
        region,
        enabled,
        gameDirectory,
      }),
    setLoaderRunning: (running, terminateProcesses) =>
      request(COMMANDS.setLoaderRunning, parseModLoaderRuntime, {
        running,
        terminateProcesses,
      }),
    subscribeRuntime: (onEvent, onError) => {
      const subscriptionId = createSubscriptionId();
      return subscribeStream({
        transport,
        streamKind: "modStudioRuntime",
        subscriptionId,
        subscribeCommand: COMMANDS.subscribeRuntime,
        unsubscribeCommand: COMMANDS.unsubscribeRuntime,
        parseEvent: parseModStudioRuntimeEvent,
        onEvent,
        onError: (error) => onError(parseModStudioCommandError(error)),
      });
    },
  };
}

export const modStudioClient = createModStudioClient();
