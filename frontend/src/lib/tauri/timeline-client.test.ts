import { describe, expect, it, vi } from "vitest";

import { createTimelineClient } from "@/lib/tauri/timeline-client";
import { MAX_STREAM_DELIVERY_BYTES } from "@/lib/tauri/stream-contract";

const encodeDelivery = (events: unknown[]): ArrayBuffer => {
  const bytes = new TextEncoder().encode(
    JSON.stringify({ streamProtocolVersion: 1, events }),
  );
  return bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
};

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
  it("subscribes with scope and releases the acknowledged stream", async () => {
    let onMessage: ((message: unknown) => void) | undefined;
    const invoke = vi.fn(async (command: string) => {
      if (command === "subscribe_timeline") {
        return {
          subscriptionId: "timeline-test",
          streamKind: "timeline",
          streamIntervalMs: 100,
          streamProtocolVersion: 1,
          streamGeneration: "1",
          maxInFlightDeliveries: 1,
          maxDeliveryBytes: MAX_STREAM_DELIVERY_BYTES,
        };
      }
      if (command === "read_stream_delivery") {
        return encodeDelivery([{ event: "snapshot", payload: EMPTY_SNAPSHOT }]);
      }
      if (command === "ack_stream_delivery") return { accepted: true };
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
    onMessage?.({
      streamProtocolVersion: 1,
      streamKind: "timeline",
      subscriptionId: "timeline-test",
      streamGeneration: "1",
      deliverySequence: "1",
    });
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
