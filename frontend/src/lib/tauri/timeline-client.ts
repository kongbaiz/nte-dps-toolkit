import { invokeCommand, type CommandError } from "@/lib/tauri/command-error";
import type { TimelineDpsViewMode } from "@/lib/tauri/generated/TimelineDpsViewMode";
import type { TimelineEvent } from "@/lib/tauri/generated/TimelineEvent";
import type { TimelineScope } from "@/lib/tauri/generated/TimelineScope";
import type { TimelineSnapshot } from "@/lib/tauri/generated/TimelineSnapshot";
import { subscribeStream } from "@/lib/tauri/stream-client";

export const timelineClient = {
  getSnapshot: (scope: TimelineScope) =>
    invokeCommand<TimelineSnapshot>("get_timeline_snapshot", { scope }),
  setPreferences: (
    scope: TimelineScope,
    bucketSeconds: number,
    viewMode: TimelineDpsViewMode,
  ) =>
    invokeCommand<TimelineSnapshot>("set_timeline_preferences", {
      scope,
      bucketSeconds,
      viewMode,
    }),
  subscribe: (
    scope: TimelineScope,
    onSnapshot: (snapshot: TimelineSnapshot) => void,
    onError: (error: CommandError) => void,
  ) =>
    subscribeStream<TimelineEvent>({
      streamKind: "timeline",
      subscribeCommand: "subscribe_timeline",
      unsubscribeCommand: "unsubscribe_timeline",
      subscribeArguments: { scope },
      onEvent: (event) => onSnapshot(event.payload),
      onError,
    }),
};
