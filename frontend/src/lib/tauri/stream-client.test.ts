import { describe, expect, it, vi } from "vitest";

import {
  subscribeStream,
  type StreamTransport,
} from "@/lib/tauri/stream-client";

const receipt = {
  streamProtocolVersion: 2,
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
    onMessage?.({ streamProtocolVersion: 2, events: [{ value: 7 }] });
    expect(onEvent).toHaveBeenCalledWith({ value: 7 });
    expect(invoke).toHaveBeenCalledWith("acknowledge_stream_delivery", {
      subscriptionId: "fixture",
      streamKind: "technical",
      streamGeneration: "1",
      deliverySequence: "1",
    });
    onMessage?.({ streamProtocolVersion: 2, events: [{ value: 8 }] });
    expect(invoke).toHaveBeenLastCalledWith("acknowledge_stream_delivery", {
      subscriptionId: "fixture",
      streamKind: "technical",
      streamGeneration: "1",
      deliverySequence: "2",
    });

    await close();
    onMessage?.({ streamProtocolVersion: 2, events: [{ value: 9 }] });
    expect(onEvent).toHaveBeenCalledTimes(2);
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

    onMessage?.({ streamProtocolVersion: 2, events: ["early"] });
    expect(onEvent).not.toHaveBeenCalled();
    expect(transport.invoke).not.toHaveBeenCalledWith(
      "acknowledge_stream_delivery",
      expect.anything(),
    );
    resolveReceipt?.(receipt);
    await flush();
    expect(onEvent).toHaveBeenCalledWith("early");
    expect(transport.invoke).toHaveBeenCalledWith(
      "acknowledge_stream_delivery",
      expect.objectContaining({ streamGeneration: "1", deliverySequence: "1" }),
    );
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
    expect(invoke).not.toHaveBeenCalledWith(
      "acknowledge_stream_delivery",
      expect.anything(),
    );
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
      invoke.mock.calls.filter(
        ([command]) => command === "unsubscribe_technical_state",
      ),
    ).toHaveLength(1);
  });

  it("fails closed and unsubscribes when acknowledgement fails", async () => {
    let onMessage: ((value: unknown) => void) | undefined;
    const failure = new Error("ack failed");
    const invoke = vi.fn(async (command: string) => {
      if (command === "subscribe_technical_state") return receipt;
      if (command === "acknowledge_stream_delivery") throw failure;
      return undefined;
    });
    const onEvent = vi.fn();
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
      onEvent,
      onError,
    });
    await flush();
    onMessage?.({ streamProtocolVersion: 2, events: ["first"] });
    await flush();
    expect(onError).toHaveBeenCalledWith(failure);
    expect(invoke).toHaveBeenCalledWith("unsubscribe_technical_state", {
      subscriptionId: "fixture",
    });
    onMessage?.({ streamProtocolVersion: 2, events: ["late"] });
    expect(onEvent).toHaveBeenCalledTimes(1);
  });

  it("does not acknowledge a delivery rejected by the consumer", async () => {
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
      onEvent: () => {
        throw new Error("consumer failed");
      },
      onError,
    });
    await flush();
    onMessage?.({ streamProtocolVersion: 2, events: ["first"] });
    await flush();
    expect(onError).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalledWith(
      "acknowledge_stream_delivery",
      expect.anything(),
    );
    expect(invoke).toHaveBeenCalledWith("unsubscribe_technical_state", {
      subscriptionId: "fixture",
    });
  });
});
