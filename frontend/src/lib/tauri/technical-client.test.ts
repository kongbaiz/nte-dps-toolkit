import { describe, expect, it, vi } from "vitest";

import { createTechnicalClient } from "./technical-client";
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

const snapshot = {
  contractVersion: 5,
  sequence: "1",
  bridgeStatus: "ready",
  adapterVersion: "0.3.6",
  windowLabel: "hud-spike",
  uptimeMs: "1200",
  streamIntervalMs: 100,
  supportedLocales: ["en"],
  window: {
    passthrough: false,
    alwaysOnTop: true,
  },
  capture: {
    phase: "idle",
    messageKey: "No live capture task right now",
    messageArguments: [],
    issue: null,
  },
  hud: {
    version: 3,
    dataState: "empty",
    config: {
      width: 380,
      moduleOrder: ["title", "summary", "status", "characters", "timeline"],
      showTitle: false,
      showTeamDps: true,
      showDuration: true,
      showTotalDamage: true,
      showCharacterRows: true,
      showDamageTaken: false,
      showAbyssHalf: false,
      showPassthroughState: false,
      showMiniTimeline: false,
    },
    summary: null,
    characters: [],
    status: {
      abyssDetected: false,
      abyssFloor: null,
      abyssHalf: null,
      abyssSuccess: false,
    },
    timeline: null,
  },
};

describe("technical client subscription", () => {
  it("routes capture lifecycle through typed snapshot commands", async () => {
    const invoke = vi.fn(() => Promise.resolve(snapshot));
    const client = createTechnicalClient({
      invoke,
      createChannel: () => ({ channel: true }),
    });

    await expect(client.startCapture()).resolves.toMatchObject({
      capture: { phase: "idle" },
    });
    await expect(client.stopCapture()).resolves.toMatchObject({
      capture: { phase: "idle" },
    });
    expect(invoke).toHaveBeenNthCalledWith(1, "start_hud_capture", undefined);
    expect(invoke).toHaveBeenNthCalledWith(2, "stop_hud_capture", undefined);
  });

  it("routes HUD reset through the dedicated reset command", async () => {
    const invoke = vi.fn(() => Promise.resolve(snapshot));
    const client = createTechnicalClient({
      invoke,
      createChannel: () => ({ channel: true }),
    });

    await expect(client.resetSession()).resolves.toMatchObject({
      hud: { dataState: "empty" },
    });
    expect(invoke).toHaveBeenCalledWith("reset_hud_session", undefined);
  });

  it("sends one typed HUD module visibility intent", async () => {
    const invoke = vi.fn(() => Promise.resolve(snapshot));
    const client = createTechnicalClient({
      invoke,
      createChannel: () => ({ channel: true }),
    });

    await expect(
      client.setModuleVisibility("timeline", true),
    ).resolves.toMatchObject({
      hud: { config: { showMiniTimeline: false } },
    });
    expect(invoke).toHaveBeenCalledWith("set_hud_module_visibility", {
      module: "timeline",
      visible: true,
    });
  });

  it("sends one typed HUD module move intent", async () => {
    const invoke = vi.fn(() => Promise.resolve(snapshot));
    const client = createTechnicalClient({
      invoke,
      createChannel: () => ({ channel: true }),
    });

    await expect(
      client.moveModule("timeline", "summary", false),
    ).resolves.toMatchObject({
      hud: {
        config: {
          moduleOrder: ["title", "summary", "status", "characters", "timeline"],
        },
      },
    });
    expect(invoke).toHaveBeenCalledWith("move_hud_module", {
      dragged: "timeline",
      target: "summary",
      insertAfter: false,
    });
  });

  it("sends one typed HUD width intent", async () => {
    const invoke = vi.fn(() => Promise.resolve(snapshot));
    const client = createTechnicalClient({
      invoke,
      createChannel: () => ({ channel: true }),
    });

    await expect(client.setWidth(512)).resolves.toMatchObject({
      hud: { config: { width: 380 } },
    });
    expect(invoke).toHaveBeenCalledWith("set_hud_width", {
      width: 512,
    });
  });

  it("unsubscribes with the receipt when the consumer is cleaned up", async () => {
    let onMessage: ((message: unknown) => void) | undefined;
    const invoke = vi.fn((command: string) => {
      if (command === "subscribe_technical_state") {
        return Promise.resolve({
          subscriptionId: "test-subscription",
          streamKind: "technical",
          streamIntervalMs: 100,
          streamProtocolVersion: 1,
          streamGeneration: "1",
          maxInFlightDeliveries: 1,
          maxDeliveryBytes: MAX_STREAM_DELIVERY_BYTES,
        });
      }
      if (command === "read_stream_delivery") {
        return Promise.resolve(
          encodeDelivery([{ event: "snapshot", payload: snapshot }]),
        );
      }
      if (command === "ack_stream_delivery") {
        return Promise.resolve({ accepted: true });
      }
      return Promise.resolve(undefined);
    });
    const client = createTechnicalClient(
      {
        invoke,
        createChannel: (handler) => {
          onMessage = handler;
          return { channel: true };
        },
      },
      () => "test-subscription",
    );
    const receive = vi.fn();
    const receiveError = vi.fn();

    const cleanup = client.subscribe(receive, receiveError);
    onMessage?.({
      streamProtocolVersion: 1,
      streamKind: "technical",
      subscriptionId: "test-subscription",
      streamGeneration: "1",
      deliverySequence: "1",
    });
    await vi.waitFor(() => expect(receive).toHaveBeenCalledTimes(1));
    await cleanup();
    onMessage?.({
      streamProtocolVersion: 1,
      streamKind: "technical",
      subscriptionId: "test-subscription",
      streamGeneration: "1",
      deliverySequence: "2",
    });

    expect(receive).toHaveBeenCalledTimes(1);
    expect(receiveError).not.toHaveBeenCalled();
    expect(invoke).toHaveBeenLastCalledWith("unsubscribe_technical_state", {
      subscriptionId: "test-subscription",
    });
  });
});
