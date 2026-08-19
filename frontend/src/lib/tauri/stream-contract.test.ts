import { describe, expect, it } from "vitest";

import {
  MAX_STREAM_DELIVERY_BYTES,
  parseStreamAckReceipt,
  parseStreamDelivery,
  parseStreamReadySignal,
  parseStreamSubscriptionReceipt,
} from "@/lib/tauri/stream-contract";

const encode = (value: unknown): ArrayBuffer => {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  return bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
};

describe("stream ACK transport contract", () => {
  it("parses the stable ready signal and acknowledged subscription receipt", () => {
    expect(
      parseStreamReadySignal({
        streamProtocolVersion: 1,
        streamKind: "mainDps",
        subscriptionId: "subscription-01",
        streamGeneration: "7",
        deliverySequence: "9",
      }),
    ).toEqual({
      streamProtocolVersion: 1,
      streamKind: "mainDps",
      subscriptionId: "subscription-01",
      streamGeneration: "7",
      deliverySequence: "9",
    });

    expect(
      parseStreamSubscriptionReceipt({
        subscriptionId: "subscription-01",
        streamKind: "mainDps",
        streamIntervalMs: 100,
        streamProtocolVersion: 1,
        streamGeneration: "7",
        maxInFlightDeliveries: 1,
        maxDeliveryBytes: MAX_STREAM_DELIVERY_BYTES,
      }),
    ).toEqual({
      subscriptionId: "subscription-01",
      streamKind: "mainDps",
      streamIntervalMs: 100,
      streamProtocolVersion: 1,
      streamGeneration: "7",
      maxInFlightDeliveries: 1,
      maxDeliveryBytes: MAX_STREAM_DELIVERY_BYTES,
    });
  });

  it("decodes a bounded ordered event batch and exact ACK receipt", () => {
    expect(
      parseStreamDelivery(
        encode({
          streamProtocolVersion: 1,
          events: [{ event: "connection" }, { event: "batch" }],
        }),
        MAX_STREAM_DELIVERY_BYTES,
      ),
    ).toEqual([{ event: "connection" }, { event: "batch" }]);
    expect(parseStreamAckReceipt({ accepted: true })).toEqual({
      accepted: true,
    });
  });

  it.each([
    { streamProtocolVersion: 2, events: [{}] },
    { streamProtocolVersion: 1, events: [] },
    { streamProtocolVersion: 1, events: [{}, {}, {}] },
  ])("rejects an invalid delivery body %#", (body) => {
    expect(() =>
      parseStreamDelivery(encode(body), MAX_STREAM_DELIVERY_BYTES),
    ).toThrow();
  });
});
