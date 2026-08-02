import { Channel, invoke } from "@tauri-apps/api/core";

import {
  packetsError,
  parsePacketsEvent,
  parsePacketsSnapshot,
  type PacketsCommandError,
  type PacketsEvent,
  type PacketsSnapshot,
} from "@/lib/tauri/packets-contract";
import {
  parseSubscriptionReceipt,
  TechnicalContractError,
} from "@/lib/tauri/technical-contract";

const COMMANDS = {
  getSnapshot: "get_packets_snapshot",
  subscribe: "subscribe_packets",
  unsubscribe: "unsubscribe_packets",
} as const;

interface PacketsTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  createChannel(onMessage: (message: unknown) => void): unknown;
}

export interface PacketsClient {
  getSnapshot(): Promise<PacketsSnapshot>;
  subscribe(
    onEvent: (event: PacketsEvent) => void,
    onError: (error: PacketsCommandError) => void,
  ): () => Promise<void>;
}

const tauriTransport: PacketsTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
  createChannel: (onMessage) => {
    const channel = new Channel<unknown>();
    channel.onmessage = onMessage;
    return channel;
  },
};

export function createPacketsClient(
  transport: PacketsTransport = tauriTransport,
  createSubscriptionId: () => string = () => crypto.randomUUID(),
): PacketsClient {
  return {
    getSnapshot: async () => {
      try {
        return parsePacketsSnapshot(
          await transport.invoke(COMMANDS.getSnapshot),
        );
      } catch (error) {
        if (error instanceof TechnicalContractError) throw error;
        throw packetsError(error);
      }
    },
    subscribe: (onEvent, onError) => {
      const subscriptionId = createSubscriptionId();
      let closed = false;
      const onChannelEvent = transport.createChannel((message) => {
        if (closed) return;
        try {
          onEvent(parsePacketsEvent(message));
        } catch (error) {
          onError(packetsError(error));
        }
      });
      const receipt = transport
        .invoke(COMMANDS.subscribe, { subscriptionId, onEvent: onChannelEvent })
        .then(parseSubscriptionReceipt)
        .catch((error: unknown) => {
          if (!closed) onError(packetsError(error));
          return undefined;
        });
      return async () => {
        if (closed) return;
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

export const packetsClient = createPacketsClient();
