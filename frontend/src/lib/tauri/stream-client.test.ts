import { describe, expect, it, vi } from "vitest";

import { subscribeAckedStream } from "@/lib/tauri/stream-client";
import { MAX_STREAM_DELIVERY_BYTES } from "@/lib/tauri/stream-contract";

const receipt = {
  subscriptionId: "subscription-01",
  streamKind: "technical",
  streamIntervalMs: 100,
  streamProtocolVersion: 1,
  streamGeneration: "7",
  maxInFlightDeliveries: 1,
  maxDeliveryBytes: MAX_STREAM_DELIVERY_BYTES,
};

const signal = {
  streamProtocolVersion: 1,
  streamKind: "technical",
  subscriptionId: "subscription-01",
  streamGeneration: "7",
  deliverySequence: "1",
};

const encode = (events: unknown[]): ArrayBuffer => {
  const bytes = new TextEncoder().encode(
    JSON.stringify({ streamProtocolVersion: 1, events }),
  );
  return bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
};

const flush = async () => {
  await Promise.resolve();
  await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
};

describe("ACKed stream client", () => {
  it("routes every desktop stream client through the shared ACK helper", () => {
    const clients = {
      "technical-client.ts": "technical",
      "diagnostics-client.ts": "diagnostics",
      "history-client.ts": "history",
      "main-dps-client.ts": "mainDps",
      "main-dps-detail-client.ts": "mainDpsDetail",
      "empty-curtain-client.ts": "emptyCurtain",
      "mod-studio-client.ts": "modStudioRuntime",
      "packets-client.ts": "packets",
      "settings-client.ts": "settings",
      "skills-client.ts": "skills",
      "timeline-client.ts": "timeline",
    } as const;
    const sources = import.meta.glob("./*-client.ts", {
      query: "?raw",
      import: "default",
      eager: true,
    }) as Record<string, string>;

    for (const [file, streamKind] of Object.entries(clients)) {
      const source = sources[`./${file}`];
      expect(source).toBeTypeOf("string");
      expect(source).toContain("subscribeAckedStream({");
      expect(source).toContain(`streamKind: "${streamKind}"`);
      expect(source).not.toContain("new Channel<");
    }

    const sharedSource = sources["./stream-client.ts"];
    expect(sharedSource).not.toMatch(
      /await pendingReceipt;\s*} catch {\s*return;/,
    );
    expect(sharedSource).toMatch(
      /await pendingReceipt;[\s\S]{1,320}await transport\.invoke\(unsubscribeCommand/,
    );
  });

  it("pulls an early signal once, applies events in order, then ACKs", async () => {
    let deliver: ((message: unknown) => void) | undefined;
    let resolveReceipt: ((value: unknown) => void) | undefined;
    const events: number[] = [];
    const invoke = vi.fn((command: string) => {
      if (command === "subscribe") {
        return new Promise<unknown>((resolve) => {
          resolveReceipt = resolve;
        });
      }
      if (command === "read_stream_delivery") {
        return Promise.resolve(encode([1, 2]));
      }
      if (command === "ack_stream_delivery") {
        return Promise.resolve({ accepted: true });
      }
      return Promise.resolve(undefined);
    });

    const unsubscribe = subscribeAckedStream({
      transport: {
        invoke,
        createChannel(onMessage) {
          deliver = onMessage;
          return { channel: true };
        },
      },
      streamKind: "technical",
      subscriptionId: "subscription-01",
      subscribeCommand: "subscribe",
      unsubscribeCommand: "unsubscribe",
      parseEvent: (value) => Number(value),
      onEvent: (event) => events.push(event),
      onError: vi.fn(),
    });

    deliver?.(signal);
    await flush();
    expect(invoke).toHaveBeenCalledWith("read_stream_delivery", {
      streamKind: "technical",
      subscriptionId: "subscription-01",
      streamGeneration: "7",
      deliverySequence: "1",
    });

    resolveReceipt?.(receipt);
    await flush();
    expect(events).toEqual([1, 2]);
    expect(invoke).toHaveBeenCalledWith("ack_stream_delivery", {
      streamKind: "technical",
      subscriptionId: "subscription-01",
      streamGeneration: "7",
      deliverySequence: "1",
    });

    await unsubscribe();
  });

  it("best-effort unsubscribes once when the subscribe response is lost", async () => {
    const onError = vi.fn();
    const invoke = vi.fn((command: string) => {
      if (command === "subscribe") {
        return Promise.reject(new Error("subscribe response lost"));
      }
      if (command === "unsubscribe") {
        return Promise.reject(new Error("unsubscribe response lost"));
      }
      return Promise.resolve(undefined);
    });
    const unsubscribe = subscribeAckedStream({
      transport: {
        invoke,
        createChannel() {
          return {};
        },
      },
      streamKind: "technical",
      subscriptionId: "subscription-01",
      subscribeCommand: "subscribe",
      unsubscribeCommand: "unsubscribe",
      parseEvent: (value) => Number(value),
      onEvent: vi.fn(),
      onError,
    });

    await vi.waitFor(() =>
      expect(
        invoke.mock.calls.filter(([command]) => command === "unsubscribe"),
      ).toHaveLength(1),
    );
    await expect(unsubscribe()).resolves.toBeUndefined();

    expect(onError).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("unsubscribe", {
      subscriptionId: "subscription-01",
    });
    expect(
      invoke.mock.calls.filter(([command]) => command === "unsubscribe"),
    ).toHaveLength(1);
  });

  it("keeps one pull in flight and a late read cannot ACK after cleanup", async () => {
    let deliver: ((message: unknown) => void) | undefined;
    let resolveRead: ((value: unknown) => void) | undefined;
    const onEvent = vi.fn();
    const invoke = vi.fn((command: string) => {
      if (command === "subscribe") return Promise.resolve(receipt);
      if (command === "read_stream_delivery") {
        return new Promise<unknown>((resolve) => {
          resolveRead = resolve;
        });
      }
      return Promise.resolve(undefined);
    });
    const unsubscribe = subscribeAckedStream({
      transport: {
        invoke,
        createChannel(onMessage) {
          deliver = onMessage;
          return {};
        },
      },
      streamKind: "technical",
      subscriptionId: "subscription-01",
      subscribeCommand: "subscribe",
      unsubscribeCommand: "unsubscribe",
      parseEvent: (value) => Number(value),
      onEvent,
      onError: vi.fn(),
    });

    deliver?.(signal);
    deliver?.({ ...signal, deliverySequence: "2" });
    await flush();
    expect(
      invoke.mock.calls.filter(
        ([command]) => command === "read_stream_delivery",
      ),
    ).toHaveLength(1);

    await unsubscribe();
    resolveRead?.(encode([1]));
    await flush();
    expect(onEvent).not.toHaveBeenCalled();
    expect(
      invoke.mock.calls.filter(
        ([command]) => command === "ack_stream_delivery",
      ),
    ).toHaveLength(0);
  });

  it("drains one queued ready signal that races the previous ACK response", async () => {
    let deliver: ((message: unknown) => void) | undefined;
    let resolveFirstAck: ((value: unknown) => void) | undefined;
    const events: number[] = [];
    const invoke = vi.fn(
      (command: string, arguments_?: Record<string, unknown>) => {
        if (command === "subscribe") return Promise.resolve(receipt);
        if (command === "read_stream_delivery") {
          return Promise.resolve(
            encode([Number(arguments_?.deliverySequence)]),
          );
        }
        if (
          command === "ack_stream_delivery" &&
          arguments_?.deliverySequence === "1"
        ) {
          return new Promise<unknown>((resolve) => {
            resolveFirstAck = resolve;
          });
        }
        if (command === "ack_stream_delivery") {
          return Promise.resolve({ accepted: true });
        }
        return Promise.resolve(undefined);
      },
    );
    const unsubscribe = subscribeAckedStream({
      transport: {
        invoke,
        createChannel(onMessage) {
          deliver = onMessage;
          return {};
        },
      },
      streamKind: "technical",
      subscriptionId: "subscription-01",
      subscribeCommand: "subscribe",
      unsubscribeCommand: "unsubscribe",
      parseEvent: (value) => Number(value),
      onEvent: (event) => events.push(event),
      onError: vi.fn(),
    });

    deliver?.(signal);
    await vi.waitFor(() =>
      expect(
        invoke.mock.calls.filter(
          ([command]) => command === "ack_stream_delivery",
        ),
      ).toHaveLength(1),
    );
    deliver?.({ ...signal, deliverySequence: "2" });
    expect(
      invoke.mock.calls.filter(
        ([command]) => command === "read_stream_delivery",
      ),
    ).toHaveLength(1);

    resolveFirstAck?.({ accepted: true });
    await vi.waitFor(() => expect(events).toEqual([1, 2]));
    expect(
      invoke.mock.calls.filter(
        ([command]) => command === "read_stream_delivery",
      ),
    ).toHaveLength(2);
    await unsubscribe();
  });

  it("fails closed after event validation fails without an ACK retry", async () => {
    let deliver: ((message: unknown) => void) | undefined;
    const onError = vi.fn();
    const invoke = vi.fn((command: string) => {
      if (command === "subscribe") return Promise.resolve(receipt);
      if (command === "read_stream_delivery") {
        return Promise.resolve(encode([{ invalid: true }]));
      }
      return Promise.resolve(undefined);
    });
    subscribeAckedStream({
      transport: {
        invoke,
        createChannel(onMessage) {
          deliver = onMessage;
          return {};
        },
      },
      streamKind: "technical",
      subscriptionId: "subscription-01",
      subscribeCommand: "subscribe",
      unsubscribeCommand: "unsubscribe",
      parseEvent() {
        throw new Error("invalid event");
      },
      onEvent: vi.fn(),
      onError,
    });

    deliver?.(signal);
    await flush();
    expect(onError).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("unsubscribe", {
      subscriptionId: "subscription-01",
    });
    expect(
      invoke.mock.calls.filter(
        ([command]) => command === "ack_stream_delivery",
      ),
    ).toHaveLength(0);
  });

  it("fails closed when the consumer callback throws before ACK", async () => {
    let deliver: ((message: unknown) => void) | undefined;
    const onError = vi.fn();
    const invoke = vi.fn((command: string) => {
      if (command === "subscribe") return Promise.resolve(receipt);
      if (command === "read_stream_delivery") {
        return Promise.resolve(encode([1]));
      }
      return Promise.resolve(undefined);
    });
    subscribeAckedStream({
      transport: {
        invoke,
        createChannel(onMessage) {
          deliver = onMessage;
          return {};
        },
      },
      streamKind: "technical",
      subscriptionId: "subscription-01",
      subscribeCommand: "subscribe",
      unsubscribeCommand: "unsubscribe",
      parseEvent: (value) => Number(value),
      onEvent() {
        throw new Error("consumer failed");
      },
      onError,
    });

    deliver?.(signal);
    await flush();
    expect(onError).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("unsubscribe", {
      subscriptionId: "subscription-01",
    });
    expect(
      invoke.mock.calls.filter(
        ([command]) => command === "ack_stream_delivery",
      ),
    ).toHaveLength(0);
  });

  it("fails closed after one rejected ACK without retrying", async () => {
    let deliver: ((message: unknown) => void) | undefined;
    const onError = vi.fn();
    const invoke = vi.fn((command: string) => {
      if (command === "subscribe") return Promise.resolve(receipt);
      if (command === "read_stream_delivery") {
        return Promise.resolve(encode([1]));
      }
      if (command === "ack_stream_delivery") {
        return Promise.reject(new Error("ACK unavailable"));
      }
      return Promise.resolve(undefined);
    });
    subscribeAckedStream({
      transport: {
        invoke,
        createChannel(onMessage) {
          deliver = onMessage;
          return {};
        },
      },
      streamKind: "technical",
      subscriptionId: "subscription-01",
      subscribeCommand: "subscribe",
      unsubscribeCommand: "unsubscribe",
      parseEvent: (value) => Number(value),
      onEvent: vi.fn(),
      onError,
    });

    deliver?.(signal);
    await flush();
    expect(onError).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("unsubscribe", {
      subscriptionId: "subscription-01",
    });
    expect(
      invoke.mock.calls.filter(
        ([command]) => command === "ack_stream_delivery",
      ),
    ).toHaveLength(1);
  });
});
