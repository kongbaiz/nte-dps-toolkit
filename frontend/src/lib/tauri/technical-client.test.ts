import { describe, expect, it, vi } from "vitest";

import { createTechnicalClient } from "./technical-client";

const snapshot = {
  contractVersion: 1,
  sequence: "1",
  bridgeStatus: "ready",
  adapterVersion: "0.3.6",
  windowLabel: "hud-spike",
  uptimeMs: "1200",
  streamIntervalMs: 750,
  supportedLocales: ["en"],
  window: {
    passthrough: false,
    alwaysOnTop: true,
  },
};

describe("technical client subscription", () => {
  it("unsubscribes with the receipt when the consumer is cleaned up", async () => {
    let onMessage: ((message: unknown) => void) | undefined;
    const invoke = vi.fn((command: string) => {
      if (command === "subscribe_technical_state") {
        return Promise.resolve({
          subscriptionId: "test-subscription",
          streamIntervalMs: 750,
        });
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
    onMessage?.({ event: "snapshot", payload: snapshot });
    await cleanup();
    onMessage?.({ event: "snapshot", payload: snapshot });

    expect(receive).toHaveBeenCalledTimes(1);
    expect(receiveError).not.toHaveBeenCalled();
    expect(invoke).toHaveBeenLastCalledWith("unsubscribe_technical_state", {
      subscriptionId: "test-subscription",
    });
  });
});
