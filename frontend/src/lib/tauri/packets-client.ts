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
  subscribeAckedStream,
  tauriAckedStreamTransport,
} from "@/lib/tauri/stream-client";

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

const tauriTransport: PacketsTransport = tauriAckedStreamTransport;

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
      return subscribeAckedStream({
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
