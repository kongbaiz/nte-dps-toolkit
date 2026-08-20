import { describe, expect, it, vi } from "vitest";

import { createSkillsClient } from "@/lib/tauri/skills-client";

const encodeDelivery = (events: unknown[]) => ({
  streamProtocolVersion: 1,
  events,
});

const SKILLS_SNAPSHOT_FIXTURE = {
  contractVersion: 1,
  generation: "1",
  scope: "all",
  hasData: false,
  totalDamage: 0,
  totalHits: "0",
  characters: [],
  rows: [],
  diagnostics: {
    unknownCharacterCount: "0",
    unknownCharacterHits: "0",
    unknownDirectionHits: "0",
    unknownDirectionDamage: 0,
    unmappedSkillRows: "0",
    unmappedSkillHits: "0",
    unmappedSkillDamage: 0,
    unmappedGameplayEffects: [],
  },
};

describe("Skills client", () => {
  it("subscribes with scope and releases the stream", async () => {
    let onMessage: ((message: unknown) => void) | undefined;
    const invoke = vi.fn(async (command: string) => {
      if (command === "subscribe_skills") {
        return {
          subscriptionId: "skills-test",
          streamKind: "skills",
          streamIntervalMs: 100,
          streamProtocolVersion: 1,
          streamGeneration: "1",
        };
      }
      return undefined;
    });
    const client = createSkillsClient(
      {
        invoke,
        createChannel: (handler) => {
          onMessage = handler;
          return { channel: true };
        },
      },
      () => "skills-test",
    );
    const received = vi.fn();
    const unsubscribe = client.subscribe("all", received, vi.fn());
    onMessage?.(
      encodeDelivery([
        { event: "snapshot", payload: SKILLS_SNAPSHOT_FIXTURE },
      ]),
    );
    await vi.waitFor(() => expect(received).toHaveBeenCalledTimes(1));
    await unsubscribe();

    expect(received).toHaveBeenCalledWith(
      expect.objectContaining({ scope: "all", hasData: false }),
    );
    expect(invoke).toHaveBeenCalledWith(
      "subscribe_skills",
      expect.objectContaining({
        subscriptionId: "skills-test",
        scope: "all",
      }),
    );
    expect(invoke).toHaveBeenCalledWith("unsubscribe_skills", {
      subscriptionId: "skills-test",
    });
  });
});
