import { describe, expect, it, vi } from "vitest";

import { createTimelineClient } from "@/lib/tauri/timeline-client";

const EMPTY_SNAPSHOT = {
  contractVersion: 2,
  generation: "1",
  scope: "all",
  viewMode: "team",
  bucketSeconds: 1,
  bucketSecondsMin: 0.2,
  bucketSecondsMax: 10,
  bucketSecondsStep: 0.1,
  hasData: false,
  duration: 0,
  totalDamage: 0,
  peakDps: 0,
  timeStopDuration: 0,
  timeStopIntervals: [],
  markers: [],
  characters: [],
  buckets: [],
  segments: [],
};

describe("Timeline client", () => {
  it("subscribes with scope and releases the acknowledged stream", async () => {
    let onMessage: ((message: unknown) => void) | undefined;
    const invoke = vi.fn(async (command: string) => {
      if (command === "subscribe_timeline") {
        return { subscriptionId: "timeline-test", streamIntervalMs: 100 };
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
    onMessage?.({ event: "snapshot", payload: EMPTY_SNAPSHOT });
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
