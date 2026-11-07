import { beforeEach, describe, expect, it, vi } from "vitest";

import { subscribeStream } from "@/lib/tauri/stream-client";

const tauri = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, arguments_?: unknown) => Promise<unknown>>(),
  send: undefined as ((event: unknown) => void) | undefined,
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: tauri.invoke,
  Channel: class {
    set onmessage(handler: (event: unknown) => void) {
      tauri.send = handler;
    }
  },
}));

const receipt = {
  subscriptionId: "fixture",
  streamKind: "technical",
  streamGeneration: "1",
  streamIntervalMs: 100,
};

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function respond(overrides: Record<string, () => Promise<unknown>> = {}) {
  tauri.invoke.mockImplementation(async (command) => {
    if (command in overrides) return overrides[command]!();
    return command === "subscribe_technical_state" ? receipt : undefined;
  });
}

function subscribe(onEvent = vi.fn(), onError = vi.fn()) {
  const close = subscribeStream({
    streamKind: "technical",
    subscribeCommand: "subscribe_technical_state",
    unsubscribeCommand: "unsubscribe_technical_state",
    onEvent,
    onError,
  });
  return { close, onEvent, onError };
}

const acknowledgements = () =>
  tauri.invoke.mock.calls.filter(
    ([command]) => command === "acknowledge_stream_delivery",
  );
const unsubscribes = () =>
  tauri.invoke.mock.calls.filter(
    ([command]) => command === "unsubscribe_technical_state",
  );

describe("stream client", () => {
  beforeEach(() => {
    tauri.invoke.mockReset();
    tauri.send = undefined;
  });

  it("acknowledges each delivery in order and stops after close", async () => {
    respond();
    const { close, onEvent } = subscribe();
    await flush();

    tauri.send?.(7);
    tauri.send?.(8);
    expect(onEvent.mock.calls).toEqual([[7], [8]]);
    expect(acknowledgements().map(([, args]) => args)).toEqual([
      expect.objectContaining({ streamGeneration: "1", deliverySequence: "1" }),
      expect.objectContaining({ streamGeneration: "1", deliverySequence: "2" }),
    ]);

    await Promise.all([close(), close()]);
    tauri.send?.(9);
    expect(onEvent).toHaveBeenCalledTimes(2);
    expect(unsubscribes()).toHaveLength(1);
  });

  it("buffers the first delivery until the receipt arrives", async () => {
    let resolveReceipt: ((value: unknown) => void) | undefined;
    respond({
      subscribe_technical_state: () =>
        new Promise((resolve) => (resolveReceipt = resolve)),
    });
    const { onEvent } = subscribe();

    tauri.send?.("early");
    expect(onEvent).not.toHaveBeenCalled();
    resolveReceipt?.(receipt);
    await flush();
    expect(onEvent).toHaveBeenCalledWith("early");
    expect(acknowledgements()).toHaveLength(1);
  });

  it("fails closed and unsubscribes when acknowledgement fails", async () => {
    respond({
      acknowledge_stream_delivery: () => Promise.reject(new Error("ack")),
    });
    const { onEvent, onError } = subscribe();
    await flush();

    tauri.send?.("first");
    await flush();
    expect(onError).toHaveBeenCalledOnce();
    expect(unsubscribes()).toHaveLength(1);
    tauri.send?.("late");
    expect(onEvent).toHaveBeenCalledTimes(1);
  });

  it("does not acknowledge a delivery rejected by the consumer", async () => {
    respond();
    const { onError } = subscribe(
      vi.fn(() => {
        throw new Error("consumer failed");
      }),
    );
    await flush();

    tauri.send?.("first");
    await flush();
    expect(onError).toHaveBeenCalledOnce();
    expect(acknowledgements()).toHaveLength(0);
    expect(unsubscribes()).toHaveLength(1);
  });
});
