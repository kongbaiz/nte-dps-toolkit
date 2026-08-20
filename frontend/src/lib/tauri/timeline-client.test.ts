import { describe, expect, it, vi } from "vitest";

import { createTimelineClient } from "@/lib/tauri/timeline-client";

const encodeDelivery = (events: unknown[]) => ({
  streamProtocolVersion: 1,
  events,
});

const EMPTY_SNAPSHOT = {
  contractVersion: 3,
  generation: "1",
  scope: "all",
  viewMode: "team",
  bucketSeconds: 1,
  effectiveBucketSeconds: 1,
  bucketSecondsMin: 0.2,
  bucketSecondsMax: 10,
  bucketSecondsStep: 0.1,
  hasData: false,
  duration: 0,
  totalDamage: 0,
  omittedRoleDamage: 0,
  omittedRoleHits: "0",
  peakDps: 0,
  timeStopDuration: 0,
  compactedTimeStopIntervals: "0",
  timeStopIntervals: [],
  markers: [],
  characters: [],
  buckets: [],
  segments: [],
};

describe("Timeline client", () => {
  it("subscribes with scope and releases the stream", async () => {
    let onMessage: ((message: unknown) => void) | undefined;
    const invoke = vi.fn(async (command: string) => {
      if (command === "subscribe_timeline") {
        return {
          subscriptionId: "timeline-test",
          streamKind: "timeline",
          streamIntervalMs: 100,
          streamProtocolVersion: 1,
          streamGeneration: "1",
        };
      }
      return undefined;
    });
    const client = createTimelineClient(
      {
        invoke,
        createChannel: (handler) => {
          onMessage = handler;
          return { channel: true };
        },
      },
      () => "timeline-test",
    );
    const received = vi.fn();
    const unsubscribe = client.subscribe("all", received, vi.fn());
    onMessage?.(
      encodeDelivery([{ event: "snapshot", payload: EMPTY_SNAPSHOT }]),
    );
    await vi.waitFor(() => expect(received).toHaveBeenCalledTimes(1));
    await unsubscribe();

    expect(received).toHaveBeenCalledWith(
      expect.objectContaining({ scope: "all", hasData: false }),
    );
    expect(invoke).toHaveBeenCalledWith(
      "subscribe_timeline",
      expect.objectContaining({
        subscriptionId: "timeline-test",
        scope: "all",
      }),
    );
    expect(invoke).toHaveBeenCalledWith("unsubscribe_timeline", {
      subscriptionId: "timeline-test",
    });
  });
});
