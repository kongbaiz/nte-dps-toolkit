import { describe, expect, it } from "vitest";

import {
  PACKETS_CONTRACT_VERSION,
  parsePacketsEvent,
  parsePacketsSnapshot,
} from "./packets-contract";

export const PACKETS_FIXTURE = {
  contractVersion: PACKETS_CONTRACT_VERSION,
  generation: "9007199254740993",
  sessionGeneration: "2",
  packetGeneration: "8",
  firstDisplaySequence: "1",
  capturePhase: "running",
  eventCount: 3,
  observedPacketCount: "9",
  packetsWithHits: "1",
  retainedPacketCount: 8,
  queuedEventCount: 2,
  displayLimit: 500,
  truncatedPacketCount: 0,
  omittedTextBytes: "0",
  omittedDeclaredIdCount: "0",
  packets: [
    {
      sequence: "8",
      timestamp: 1_786_000_000.125,
      source: "127.0.0.1:3010",
      destination: "127.0.0.1:7777",
      direction: "outgoing",
      payloadLen: 128,
      declaredIds: [1076],
      parsedHits: 1,
      note: "accepted",
      decodedText: "GameplayEffect Shinku",
      omittedTextBytes: "0",
      omittedDeclaredIdCount: "0",
    },
  ],
};

describe("Packets contract", () => {
  it("parses bounded debug rows and keeps unsafe generations as strings", () => {
    const snapshot = parsePacketsSnapshot(PACKETS_FIXTURE);

    expect(snapshot.generation).toBe("9007199254740993");
    expect(snapshot.packets[0]).toMatchObject({
      sequence: "8",
      declaredIds: [1076],
      parsedHits: 1,
    });
  });

  it("parses append events with explicit merge semantics", () => {
    expect(
      parsePacketsEvent({ event: "append", payload: PACKETS_FIXTURE }),
    ).toMatchObject({ mode: "append", snapshot: { packetGeneration: "8" } });
  });

  it("rejects oversized packet windows and malformed counters", () => {
    expect(() =>
      parsePacketsSnapshot({ ...PACKETS_FIXTURE, displayLimit: 501 }),
    ).toThrow(/displayLimit/);
    expect(() =>
      parsePacketsSnapshot({ ...PACKETS_FIXTURE, observedPacketCount: 9 }),
    ).toThrow(/decimal string/);
    expect(() =>
      parsePacketsSnapshot({ ...PACKETS_FIXTURE, firstDisplaySequence: "9" }),
    ).toThrow(/precedes firstDisplaySequence/);
  });

  it("validates UTF-8 byte limits and explicit truncation metadata", () => {
    const packet = PACKETS_FIXTURE.packets[0];
    expect(() =>
      parsePacketsSnapshot({
        ...PACKETS_FIXTURE,
        packets: [{ ...packet, source: "界".repeat(171) }],
      }),
    ).toThrow(/UTF-8 bytes/);
    expect(() =>
      parsePacketsSnapshot({
        ...PACKETS_FIXTURE,
        truncatedPacketCount: 1,
      }),
    ).toThrow(/omission metadata/);

    expect(
      parsePacketsSnapshot({
        ...PACKETS_FIXTURE,
        truncatedPacketCount: 1,
        omittedTextBytes: "3",
        packets: [{ ...packet, source: "", omittedTextBytes: "3" }],
      }),
    ).toMatchObject({
      truncatedPacketCount: 1,
      omittedTextBytes: "3",
      packets: [{ source: "", omittedTextBytes: "3" }],
    });
  });
});
