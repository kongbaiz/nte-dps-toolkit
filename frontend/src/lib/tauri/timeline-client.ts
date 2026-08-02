import { Channel, invoke } from "@tauri-apps/api/core";

import {
  parseTimelineEvent,
  parseTimelineSnapshot,
  timelineError,
  type TimelineCommandError,
  type TimelineCurveMode,
  type TimelineScope,
  type TimelineSnapshot,
} from "@/lib/tauri/timeline-contract";
import {
  parseSubscriptionReceipt,
  TechnicalContractError,
} from "@/lib/tauri/technical-contract";

const COMMANDS = {
  getSnapshot: "get_timeline_snapshot",
  setPreferences: "set_timeline_preferences",
  subscribe: "subscribe_timeline",
  unsubscribe: "unsubscribe_timeline",
} as const;

interface TimelineTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  createChannel(onMessage: (message: unknown) => void): unknown;
}

export interface TimelineClient {
  getSnapshot(scope: TimelineScope): Promise<TimelineSnapshot>;
  setPreferences(
    scope: TimelineScope,
    bucketSeconds: number,
    viewMode: TimelineCurveMode,
  ): Promise<TimelineSnapshot>;
  subscribe(
    scope: TimelineScope,
    onSnapshot: (snapshot: TimelineSnapshot) => void,
    onError: (error: TimelineCommandError) => void,
  ): () => Promise<void>;
}

const tauriTransport: TimelineTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
  createChannel: (onMessage) => {
    const channel = new Channel<unknown>();
    channel.onmessage = onMessage;
    return channel;
  },
};

export function createTimelineClient(
  transport: TimelineTransport = tauriTransport,
  createSubscriptionId: () => string = () => crypto.randomUUID(),
): TimelineClient {
  const run = async (
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<TimelineSnapshot> => {
    try {
      return parseTimelineSnapshot(await transport.invoke(command, arguments_));
    } catch (error) {
      if (error instanceof TechnicalContractError) throw error;
      throw timelineError(error);
    }
  };
  return {
    getSnapshot: (scope) => run(COMMANDS.getSnapshot, { scope }),
    setPreferences: (scope, bucketSeconds, viewMode) =>
      run(COMMANDS.setPreferences, { scope, bucketSeconds, viewMode }),
    subscribe: (scope, onSnapshot, onError) => {
      const subscriptionId = createSubscriptionId();
      let closed = false;
      const onEvent = transport.createChannel((message) => {
        if (closed) return;
        try {
          onSnapshot(parseTimelineEvent(message));
        } catch (error) {
          onError(timelineError(error));
        }
      });
      const receipt = transport
        .invoke(COMMANDS.subscribe, { subscriptionId, scope, onEvent })
        .then(parseSubscriptionReceipt)
        .catch((error: unknown) => {
          if (!closed) onError(timelineError(error));
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

export const timelineClient = createTimelineClient();
