import {
  parseTimelineEvent,
  parseTimelineSnapshot,
  timelineError,
  type TimelineCommandError,
  type TimelineCurveMode,
  type TimelineScope,
  type TimelineSnapshot,
} from "@/lib/tauri/timeline-contract";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";
import {
  subscribeAckedStream,
  tauriAckedStreamTransport,
} from "@/lib/tauri/stream-client";

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

const tauriTransport: TimelineTransport = tauriAckedStreamTransport;

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
      return subscribeAckedStream({
        transport,
        streamKind: "timeline",
        subscriptionId,
        subscribeCommand: COMMANDS.subscribe,
        unsubscribeCommand: COMMANDS.unsubscribe,
        subscribeArguments: { scope },
        parseEvent: parseTimelineEvent,
        onEvent: onSnapshot,
        onError: (error) => onError(timelineError(error)),
      });
    },
  };
}

export const timelineClient = createTimelineClient();
