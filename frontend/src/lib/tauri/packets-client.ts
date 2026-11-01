import { invokeCommand, type CommandError } from "@/lib/tauri/command-error";
import type { PacketsEvent } from "@/lib/tauri/generated/PacketsEvent";
import type { PacketsSnapshot } from "@/lib/tauri/generated/PacketsSnapshot";
import { subscribeStream } from "@/lib/tauri/stream-client";

export const packetsClient = {
  getSnapshot: () => invokeCommand<PacketsSnapshot>("get_packets_snapshot"),
  subscribe: (
    onEvent: (event: PacketsEvent) => void,
    onError: (error: CommandError) => void,
  ) =>
    subscribeStream<PacketsEvent>({
      streamKind: "packets",
      subscribeCommand: "subscribe_packets",
      unsubscribeCommand: "unsubscribe_packets",
      onEvent,
      onError,
    }),
};
