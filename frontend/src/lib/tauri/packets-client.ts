import {
  packetsError,
  parsePacketsEvent,
  parsePacketsSnapshot,
  type PacketsCommandError,
  type PacketsEvent,
  type PacketsSnapshot,
} from "@/lib/tauri/packets-contract";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";
import {
  subscribeStream,
  tauriStreamTransport,
  type StreamTransport,
} from "@/lib/tauri/stream-client";

const COMMANDS = {
  getSnapshot: "get_packets_snapshot",
  subscribe: "subscribe_packets",
  unsubscribe: "unsubscribe_packets",
} as const;

export interface PacketsClient {
  getSnapshot(): Promise<PacketsSnapshot>;
  subscribe(
    onEvent: (event: PacketsEvent) => void,
    onError: (error: PacketsCommandError) => void,
  ): () => Promise<void>;
}

export function createPacketsClient(
  transport: StreamTransport = tauriStreamTransport,
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
      return subscribeStream({
        transport,
        streamKind: "packets",
        subscriptionId,
        subscribeCommand: COMMANDS.subscribe,
        unsubscribeCommand: COMMANDS.unsubscribe,
        parseEvent: parsePacketsEvent,
        onEvent,
        onError: (error) => onError(packetsError(error)),
      });
    },
  };
}

export const packetsClient = createPacketsClient();
