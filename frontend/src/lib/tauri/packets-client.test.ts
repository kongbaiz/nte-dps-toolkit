import { describe, expect, it, vi } from "vitest";

import { createPacketsClient } from "./packets-client";
import { MAX_STREAM_DELIVERY_BYTES } from "./stream-contract";

const encodeDelivery = (events: unknown[]): ArrayBuffer => {
  const bytes = new TextEncoder().encode(
    JSON.stringify({ streamProtocolVersion: 1, events }),
  );
  return bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
};

const PACKETS_FIXTURE = {
  contractVersion: 1,
  generation: "7",
  sessionGeneration: "2",
  packetGeneration: "5",
  capturePhase: "running",
  eventCount: 3,
  observedPacketCount: "5",
  packetsWithHits: "1",
  retainedPacketCount: 1,
  queuedEventCount: 0,
  displayLimit: 500,
  packets: [
    {
      sequence: "5",
      timestamp: 1.25,
      source: "source",
      destination: "destination",
      direction: "outgoing",
      payloadLen: 128,
      declaredIds: [1076],
      parsedHits: 1,
      note: "",
      decodedText: "decoded",
    },
  ],
};

describe("Packets client", () => {
  it("subscribes through a Channel and releases the acknowledged stream", async () => {
    let onMessage: ((message: unknown) => void) | undefined;
    const invoke = vi.fn(async (command: string) => {
      if (command === "subscribe_packets") {
        return {
          subscriptionId: "packets-test",
          streamKind: "packets",
          streamIntervalMs: 100,
          streamProtocolVersion: 1,
          streamGeneration: "1",
          maxInFlightDeliveries: 1,
          maxDeliveryBytes: MAX_STREAM_DELIVERY_BYTES,
        };
      }
      if (command === "read_stream_delivery") {
        return encodeDelivery([
          { event: "snapshot", payload: PACKETS_FIXTURE },
        ]);
      }
      if (command === "ack_stream_delivery") return { accepted: true };
      if (command === "get_packets_snapshot") return PACKETS_FIXTURE;
      return undefined;
    });
    const client = createPacketsClient(
      {
        invoke,
        createChannel: (handler) => {
          onMessage = handler;
          return { channel: true };
        },
      },
      () => "packets-test",
    );
    const received = vi.fn();
    const unsubscribe = client.subscribe(received, vi.fn());
    onMessage?.({
      streamProtocolVersion: 1,
      streamKind: "packets",
      subscriptionId: "packets-test",
      streamGeneration: "1",
      deliverySequence: "1",
    });
    await vi.waitFor(() => expect(received).toHaveBeenCalledTimes(1));
    await unsubscribe();

    expect(received).toHaveBeenCalledWith(
      expect.objectContaining({ mode: "replace" }),
    );
    expect(invoke).toHaveBeenCalledWith(
      "subscribe_packets",
      expect.objectContaining({ subscriptionId: "packets-test" }),
    );
    expect(invoke).toHaveBeenCalledWith("unsubscribe_packets", {
      subscriptionId: "packets-test",
    });
  });
});
