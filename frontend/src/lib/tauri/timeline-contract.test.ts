import { describe, expect, it } from "vitest";

import {
  parseTimelineEvent,
  parseTimelineSnapshot,
} from "@/lib/tauri/timeline-contract";

const SNAPSHOT = {
  contractVersion: 2,
  generation: "9007199254740992",
  scope: "all",
  viewMode: "characters",
  bucketSeconds: 1,
  bucketSecondsMin: 0.2,
  bucketSecondsMax: 10,
  bucketSecondsStep: 0.1,
  hasData: true,
  duration: 1,
  totalDamage: 100,
  peakDps: 100,
  timeStopDuration: 0,
  timeStopIntervals: [],
  markers: [],
  characters: [
    { id: 1, name: "Character", color: "#123abc", totalDamage: 100 },
  ],
  buckets: [
    {
      start: 0,
      end: 1,
      teamDps: 100,
      damage: 100,
      hits: "2",
      cumulativeDamage: 100,
      roles: [{ characterId: 1, dps: 100 }],
    },
  ],
  segments: [{ start: 0, end: 1, dps: 100 }],
};

describe("Timeline contract", () => {
  it("keeps decimal generations and hit counts as strings", () => {
    const snapshot = parseTimelineSnapshot(SNAPSHOT);
    expect(snapshot.generation).toBe("9007199254740992");
    expect(snapshot.buckets[0].hits).toBe("2");
    expect(snapshot.timeStopDuration).toBe(0);
  });

  it("parses tagged snapshot events and rejects inconsistent ranges", () => {
    expect(
      parseTimelineEvent({ event: "snapshot", payload: SNAPSHOT }).scope,
    ).toBe("all");
    expect(() =>
      parseTimelineSnapshot({ ...SNAPSHOT, bucketSeconds: 20 }),
    ).toThrow(/range is inconsistent/);
  });
});
