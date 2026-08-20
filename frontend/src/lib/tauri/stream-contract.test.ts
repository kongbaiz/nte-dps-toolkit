import { describe, expect, it } from "vitest";

import {
  parseStreamDelivery,
  parseStreamSubscriptionReceipt,
} from "@/lib/tauri/stream-contract";

describe("stream contract", () => {
  it("parses the compact subscription receipt", () => {
    expect(
      parseStreamSubscriptionReceipt({
        streamProtocolVersion: 1,
        subscriptionId: "main_01",
        streamKind: "mainDps",
        streamGeneration: "7",
        streamIntervalMs: 100,
      }),
    ).toEqual({
      streamProtocolVersion: 1,
      subscriptionId: "main_01",
      streamKind: "mainDps",
      streamGeneration: "7",
      streamIntervalMs: 100,
    });
  });

  it("parses direct channel deliveries", () => {
    expect(
      parseStreamDelivery({
        streamProtocolVersion: 1,
        events: [{ event: "snapshot" }],
      }),
    ).toEqual([{ event: "snapshot" }]);
  });

  it("rejects invalid versions and unbounded batches", () => {
    expect(() =>
      parseStreamDelivery({ streamProtocolVersion: 2, events: [1] }),
    ).toThrow("unsupported stream protocol version");
    expect(() =>
      parseStreamDelivery({ streamProtocolVersion: 1, events: [] }),
    ).toThrow("must contain 1..2 events");
    expect(() =>
      parseStreamDelivery({ streamProtocolVersion: 1, events: [1, 2, 3] }),
    ).toThrow("must contain 1..2 events");
  });
});
