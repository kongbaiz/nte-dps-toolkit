import { Channel, invoke } from "@tauri-apps/api/core";

import {
  parseSubscriptionReceipt,
  parseTechnicalCommandError,
  parseTechnicalEvent,
  parseTechnicalSnapshot,
  TechnicalContractError,
  type TechnicalCommandError,
  type TechnicalSnapshot,
} from "@/lib/tauri/technical-contract";

const COMMANDS = {
  getSnapshot: "get_technical_snapshot",
  setAlwaysOnTop: "set_hud_always_on_top",
  setPassthrough: "set_hud_passthrough",
  subscribe: "subscribe_technical_state",
  unsubscribe: "unsubscribe_technical_state",
} as const;

interface TechnicalTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  createChannel(onMessage: (message: unknown) => void): unknown;
}

export interface TechnicalClient {
  getSnapshot(): Promise<TechnicalSnapshot>;
  setAlwaysOnTop(enabled: boolean): Promise<TechnicalSnapshot>;
  setPassthrough(enabled: boolean): Promise<TechnicalSnapshot>;
  subscribe(
    onSnapshot: (snapshot: TechnicalSnapshot) => void,
    onError: (error: TechnicalCommandError) => void,
  ): () => Promise<void>;
}

const tauriTransport: TechnicalTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
  createChannel: (onMessage) => {
    const channel = new Channel<unknown>();
    channel.onmessage = onMessage;
    return channel;
  },
};

export function createTechnicalClient(
  transport: TechnicalTransport = tauriTransport,
  createSubscriptionId: () => string = () => crypto.randomUUID(),
): TechnicalClient {
  async function snapshotCommand(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<TechnicalSnapshot> {
    try {
      return parseTechnicalSnapshot(
        await transport.invoke(command, arguments_),
      );
    } catch (error) {
      if (error instanceof TechnicalContractError) {
        throw error;
      }
      throw parseTechnicalCommandError(error);
    }
  }

  return {
    getSnapshot: () => snapshotCommand(COMMANDS.getSnapshot),
    setAlwaysOnTop: (enabled) =>
      snapshotCommand(COMMANDS.setAlwaysOnTop, { enabled }),
    setPassthrough: (enabled) =>
      snapshotCommand(COMMANDS.setPassthrough, { enabled }),
    subscribe: (onSnapshot, onError) => {
      const subscriptionId = createSubscriptionId();
      let closed = false;
      const onMessage = (message: unknown) => {
        if (closed) {
          return;
        }

        try {
          onSnapshot(parseTechnicalEvent(message).payload);
        } catch (error) {
          onError(parseTechnicalCommandError(error));
        }
      };
      const onEvent = transport.createChannel(onMessage);
      const receipt = transport
        .invoke(COMMANDS.subscribe, { subscriptionId, onEvent })
        .then(parseSubscriptionReceipt)
        .catch((error: unknown) => {
          if (!closed) {
            onError(parseTechnicalCommandError(error));
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
          await transport.invoke(COMMANDS.unsubscribe, {
            subscriptionId: activeReceipt.subscriptionId,
          });
        }
      };
    },
  };
}

export const technicalClient = createTechnicalClient();
