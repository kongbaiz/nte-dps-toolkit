import { describe, expect, it, vi } from "vitest";

import { subscribeStream, type StreamTransport } from "@/lib/tauri/stream-client";

const receipt = {
  streamProtocolVersion: 1,
  subscriptionId: "fixture",
  streamKind: "technical",
  streamGeneration: "1",
  streamIntervalMs: 100,
};

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("stream client", () => {
  it("subscribes with one direct Tauri Channel", async () => {
    let onMessage: ((value: unknown) => void) | undefined;
    const channel = {};
    const invoke = vi.fn(async (command: string) => {
      if (command === "subscribe_technical_state") return receipt;
      return undefined;
    });
    const transport: StreamTransport = {
      invoke,
      createChannel(callback) {
        onMessage = callback;
        return channel;
      },
    };
    const onEvent = vi.fn();
    const close = subscribeStream({
      transport,
      streamKind: "technical",
      subscriptionId: "fixture",
      subscribeCommand: "subscribe_technical_state",
      unsubscribeCommand: "unsubscribe_technical_state",
      parseEvent: (value) => value,
      onEvent,
      onError: vi.fn(),
    });
    await flush();

    expect(invoke).toHaveBeenCalledWith("subscribe_technical_state", {
      subscriptionId: "fixture",
      onEvent: channel,
    });
    onMessage?.({ streamProtocolVersion: 1, events: [{ value: 7 }] });
    expect(onEvent).toHaveBeenCalledWith({ value: 7 });

    await close();
    expect(invoke).toHaveBeenCalledWith("unsubscribe_technical_state", {
      subscriptionId: "fixture",
    });
  });

  it("buffers the first delivery until the receipt is validated", async () => {
    let onMessage: ((value: unknown) => void) | undefined;
    let resolveReceipt: ((value: unknown) => void) | undefined;
    const subscribe = new Promise<unknown>((resolve) => {
      resolveReceipt = resolve;
    });
    const transport: StreamTransport = {
      invoke: vi.fn((command) =>
        command === "subscribe_technical_state"
          ? subscribe
          : Promise.resolve(undefined),
      ),
      createChannel(callback) {
        onMessage = callback;
        return {};
      },
    };
    const onEvent = vi.fn();
    subscribeStream({
      transport,
      streamKind: "technical",
      subscriptionId: "fixture",
      subscribeCommand: "subscribe_technical_state",
      unsubscribeCommand: "unsubscribe_technical_state",
      parseEvent: (value) => value,
      onEvent,
      onError: vi.fn(),
    });

    onMessage?.({ streamProtocolVersion: 1, events: ["early"] });
    expect(onEvent).not.toHaveBeenCalled();
    resolveReceipt?.(receipt);
    await flush();
    expect(onEvent).toHaveBeenCalledWith("early");
  });

  it("fails closed on an invalid delivery", async () => {
    let onMessage: ((value: unknown) => void) | undefined;
    const invoke = vi.fn(async (command: string) =>
      command === "subscribe_technical_state" ? receipt : undefined,
    );
    const onError = vi.fn();
    subscribeStream({
      transport: {
        invoke,
        createChannel(callback) {
          onMessage = callback;
          return {};
        },
      },
      streamKind: "technical",
      subscriptionId: "fixture",
      subscribeCommand: "subscribe_technical_state",
      unsubscribeCommand: "unsubscribe_technical_state",
      parseEvent: (value) => value,
      onEvent: vi.fn(),
      onError,
    });
    await flush();

    onMessage?.({ streamProtocolVersion: 99, events: ["invalid"] });
    await flush();
    expect(onError).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenCalledWith("unsubscribe_technical_state", {
      subscriptionId: "fixture",
    });
  });

  it("makes close idempotent", async () => {
    const invoke = vi.fn(async (command: string) =>
      command === "subscribe_technical_state" ? receipt : undefined,
    );
    const close = subscribeStream({
      transport: { invoke, createChannel: () => ({}) },
      streamKind: "technical",
      subscriptionId: "fixture",
      subscribeCommand: "subscribe_technical_state",
      unsubscribeCommand: "unsubscribe_technical_state",
      parseEvent: (value) => value,
      onEvent: vi.fn(),
      onError: vi.fn(),
    });

    await Promise.all([close(), close()]);
    expect(
      invoke.mock.calls.filter(([command]) =>
        command === "unsubscribe_technical_state",
      ),
    ).toHaveLength(1);
  });
});
