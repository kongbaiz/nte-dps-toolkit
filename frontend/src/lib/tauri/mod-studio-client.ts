import { Channel, invoke } from "@tauri-apps/api/core";

import {
  ModStudioContractError,
  parseModStudioCommandError,
  parseModStudioDocument,
  parseModStudioRuntimeEvent,
  parseModStudioSdkSchema,
  parseModStudioSubscriptionReceipt,
  parseModStudioWorkspace,
  type ModStudioCommandError,
  type ModStudioDocumentSnapshot,
  type ModStudioRuntimeEvent,
  type ModStudioSdkSchemaSnapshot,
  type ModStudioWorkspaceSnapshot,
} from "@/lib/tauri/mod-studio-contract";

const COMMANDS = {
  getDocument: "get_mod_studio_document",
  getSdkSchema: "get_mod_studio_sdk_schema",
  getWorkspace: "get_mod_studio_workspace",
  saveDocument: "save_mod_studio_document",
  setEnabled: "set_mod_studio_document_enabled",
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
  getDocument(id: string): Promise<ModStudioDocumentSnapshot>;
  getSdkSchema(): Promise<ModStudioSdkSchemaSnapshot>;
  getWorkspace(): Promise<ModStudioWorkspaceSnapshot>;
  saveDocument(id: string, source: string): Promise<ModStudioDocumentSnapshot>;
  setEnabled(id: string, enabled: boolean): Promise<ModStudioWorkspaceSnapshot>;
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
    getDocument: (id) =>
      request(COMMANDS.getDocument, parseModStudioDocument, { id }),
    getSdkSchema: () => request(COMMANDS.getSdkSchema, parseModStudioSdkSchema),
    getWorkspace: () => request(COMMANDS.getWorkspace, parseModStudioWorkspace),
    saveDocument: (id, source) =>
      request(COMMANDS.saveDocument, parseModStudioDocument, { id, source }),
    setEnabled: (id, enabled) =>
      request(COMMANDS.setEnabled, parseModStudioWorkspace, { id, enabled }),
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
