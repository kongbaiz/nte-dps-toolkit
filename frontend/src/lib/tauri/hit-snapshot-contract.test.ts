import { describe, expect, it } from "vitest";
import { hitSnapshotFixture } from "../../../dev/hit-snapshot-fixture";
import {
  parseHitSnapshotReply,
  parseEffectCounts,
} from "./hit-snapshot-contract";
import {
  criticalTranslationKey,
  effectCountsText,
} from "@/features/main-dps/main-dps-detail-model";
const snapshotReference = "9001";
const reply = () => ({
  contractVersion: 1,
  hitId: "hit-1",
  snapshotKey: snapshotReference,
  snapshot: structuredClone(hitSnapshotFixture),
});
describe("frozen hit snapshot contract", () => {
  it("preserves true, false, null, zero and decimal identities", () => {
    for (const critical of [true, false, null]) {
      const input = reply();
      input.snapshot.critical = critical;
      const snapshot = parseHitSnapshotReply(input, "hit-1", snapshotReference);
      expect(snapshot.critical).toBe(critical);
      expect(snapshot.key).toBe("preview-capture:1");
      expect(snapshot.data?.attackerAttributes?.values?.hp.value).toBe(0);
      expect(snapshot.data?.attackerAttributes?.values?.crit.value).toBeNull();
      expect(snapshot.data?.attackerAttributes?.id).toBe("9007199254740993");
    }
    expect([true, false, null].map(criticalTranslationKey)).toEqual([
      "Yes",
      "No",
      "Unknown value",
    ]);
    expect(
      effectCountsText({ positive: 5, negative: 0, other: 15, complete: true }),
    ).toBe("5 / 0 (+15)");
    expect(effectCountsText(null)).toBeNull();
  });
  it("rejects wrong hit identity, unsafe values and oversized effects", () => {
    expect(() =>
      parseHitSnapshotReply(reply(), "other", snapshotReference),
    ).toThrow();
    expect(() => parseHitSnapshotReply(reply(), "hit-1", "other")).toThrow();
    const input = reply();
    input.snapshot.data!.attackerAttributes!.values!.attack.value = Infinity;
    expect(() =>
      parseHitSnapshotReply(input, "hit-1", snapshotReference),
    ).toThrow();
    const big = reply();
    big.snapshot.data!.attackerEffects!.effects = Array(513).fill(
      big.snapshot.data!.attackerEffects!.effects![0],
    );
    expect(() =>
      parseHitSnapshotReply(big, "hit-1", snapshotReference),
    ).toThrow();
    expect(() =>
      parseEffectCounts({
        positive: 513,
        negative: 0,
        other: 0,
        complete: true,
      }),
    ).toThrow();
  });
  it("does not turn an unavailable snapshot into a fabricated empty observation", () => {
    const input = reply();
    input.snapshot.retention = "budget_exceeded";
    input.snapshot.data = null;
    expect(
      parseHitSnapshotReply(input, "hit-1", snapshotReference).data,
    ).toBeNull();
    input.snapshot.retention = "available";
    expect(() =>
      parseHitSnapshotReply(input, "hit-1", snapshotReference),
    ).toThrow();
  });
});
