import { describe, expect, it, vi } from "vitest";

import { createEmptyCurtainClient } from "@/lib/tauri/empty-curtain-client";
import { EMPTY_CURTAIN_FIXTURE } from "@/lib/tauri/empty-curtain-contract.test";
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

describe("Console equipment client", () => {
  it("releases the acknowledged coalesced stream", async () => {
    let onMessage: ((message: unknown) => void) | undefined;
    const invoke = vi.fn(async (command: string) => {
      if (command === "subscribe_empty_curtain") {
        return {
          subscriptionId: "empty-curtain-test",
          streamKind: "emptyCurtain",
          streamIntervalMs: 100,
          streamProtocolVersion: 1,
          streamGeneration: "1",
          maxInFlightDeliveries: 1,
          maxDeliveryBytes: MAX_STREAM_DELIVERY_BYTES,
        };
      }
      if (command === "read_stream_delivery") {
        return encodeDelivery([
          { event: "snapshot", payload: EMPTY_CURTAIN_FIXTURE },
        ]);
      }
      if (command === "ack_stream_delivery") return { accepted: true };
      return undefined;
    });
    const client = createEmptyCurtainClient(
      {
        invoke,
        createChannel: (handler) => {
          onMessage = handler;
          return { channel: true };
        },
      },
      () => "empty-curtain-test",
    );
    const received = vi.fn();
    const unsubscribe = client.subscribe(received, vi.fn());
    onMessage?.({
      streamProtocolVersion: 1,
      streamKind: "emptyCurtain",
      subscriptionId: "empty-curtain-test",
      streamGeneration: "1",
      deliverySequence: "1",
    });
    await vi.waitFor(() => expect(received).toHaveBeenCalledTimes(1));
    await unsubscribe();

    expect(received).toHaveBeenCalledWith(
      expect.objectContaining({ hasData: true }),
    );
    expect(invoke).toHaveBeenCalledWith(
      "subscribe_empty_curtain",
      expect.objectContaining({ subscriptionId: "empty-curtain-test" }),
    );
    expect(invoke).toHaveBeenCalledWith("unsubscribe_empty_curtain", {
      subscriptionId: "empty-curtain-test",
    });
  });

  it("sends bounded equip arguments through the typed command", async () => {
    const invoke = vi.fn(async () => EMPTY_CURTAIN_FIXTURE);
    const client = createEmptyCurtainClient({
      invoke,
      createChannel: () => ({}),
    });
    await client.manageItem({
      item: { slot: 3, serial: 4 },
      action: "equip",
      character: { slot: 1, serial: 2 },
      position: { row: 0, column: 1 },
    });
    expect(invoke).toHaveBeenCalledWith("manage_empty_curtain_item", {
      item: { slot: 3, serial: 4 },
      action: "equip",
      character: { slot: 1, serial: 2 },
      row: 0,
      column: 1,
    });
  });
});
