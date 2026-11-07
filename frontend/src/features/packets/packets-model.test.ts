import { describe, expect, it } from "vitest";

import type { PacketSnapshot } from "@/lib/tauri/generated/PacketSnapshot";
import type { PacketsSnapshot } from "@/lib/tauri/generated/PacketsSnapshot";

import {
  compareDecimal,
  mergePacketsEvent,
  normalizePacketSearch,
  packetMatches,
  packetsContentKind,
} from "./packets-model";

function packet(sequence: string, parsedHits = 0): PacketSnapshot {
  return {
    sequence,
    timestamp: Number(sequence),
    source: `source-${sequence}`,
    destination: "destination",
    direction: "outgoing",
    payloadLen: 128,
    declaredIds: [1076],
    parsedHits,
    note: "accepted",
    decodedText: "GameplayEffect Shinku",
    omittedTextBytes: "0",
    omittedDeclaredIdCount: "0",
  };
}

function snapshot(
  generation: string,
  packets: PacketSnapshot[],
): PacketsSnapshot {
  return {
    generation,
    sessionGeneration: "1",
    packetGeneration: packets.at(-1)?.sequence ?? "0",
    firstDisplaySequence: packets.at(0)?.sequence ?? "1",
    capturePhase: "running",
    eventCount: 0,
    observedPacketCount: generation,
    packetsWithHits: "0",
    retainedPacketCount: packets.length,
    queuedEventCount: 0,
    displayLimit: 3,
    truncatedPacketCount: 0,
    omittedTextBytes: "0",
    omittedDeclaredIdCount: "0",
    packets,
  };
}

describe("Packets view model", () => {
  it("selects loading, error, empty, filtered-empty and list states", () => {
    expect(packetsContentKind("loading", 0, 0)).toBe("loading");
    expect(packetsContentKind("error", 0, 0)).toBe("error");
    expect(packetsContentKind("ready", 0, 0)).toBe("empty");
    expect(packetsContentKind("ready", 4, 0)).toBe("filtered-empty");
    expect(packetsContentKind("ready", 4, 2)).toBe("list");
  });

  it("merges ordered append batches, removes duplicates and keeps the display bound", () => {
    const current = snapshot("2", [packet("1"), packet("2")]);
    const incoming = snapshot("4", [packet("2"), packet("3"), packet("4")]);

    expect(
      mergePacketsEvent(current, {
        event: "append",
        payload: incoming,
      }).packets.map((item) => item.sequence),
    ).toEqual(["2", "3", "4"]);
  });

  it("uses the Rust first-display sequence", () => {
    const current = snapshot("2", [packet("1"), packet("2")]);
    const incoming = snapshot("4", [packet("3"), packet("4")]);
    expect(incoming.firstDisplaySequence).toBe("3");
    expect(
      mergePacketsEvent(current, {
        event: "append",
        payload: incoming,
      }).packets.map((item) => item.sequence),
    ).toEqual(["3", "4"]);
  });

  it("ignores stale batches and replaces a changed session", () => {
    const current = snapshot("5", [packet("5")]);
    expect(
      mergePacketsEvent(current, {
        event: "append",
        payload: snapshot("4", [packet("4")]),
      }),
    ).toBe(current);

    const nextSession = {
      ...snapshot("6", [packet("1")]),
      sessionGeneration: "2",
    };
    expect(
      mergePacketsEvent(current, { event: "append", payload: nextSession }),
    ).toBe(nextSession);
  });

  it("filters by hits, endpoints, identifiers and decoded protocol text", () => {
    const hit = packet("8", 1);
    expect(packetMatches(hit, normalizePacketSearch("SOURCE-8"), false)).toBe(
      true,
    );
    expect(packetMatches(hit, "1076", false)).toBe(true);
    expect(packetMatches(hit, "shinku", false)).toBe(true);
    expect(packetMatches(packet("9"), "", true)).toBe(false);
  });

  it("orders large decimal sequences without Number conversion", () => {
    expect(compareDecimal("9", "10")).toBeLessThan(0);
    expect(compareDecimal("18446744073709551615", "10")).toBeGreaterThan(0);
  });
});
