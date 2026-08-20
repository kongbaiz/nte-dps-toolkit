import { describe, expect, it } from "vitest";

import type { MainDpsDetailSnapshot } from "@/lib/tauri/main-dps-detail-contract";

import {
  DEFAULT_DETAIL_COLUMNS,
  detailVisibleRowRange,
  mergeLiveDetailSnapshot,
  mergePagedDetailSnapshot,
  resetDetailColumnWidths,
  setDetailColumnVisible,
  setDetailColumnWidth,
} from "./main-dps-detail-model";

describe("main DPS detail model", () => {
  it("clamps persisted widths and includes the time visibility setting", () => {
    const hidden = setDetailColumnVisible(
      DEFAULT_DETAIL_COLUMNS,
      "time",
      false,
    );
    expect(hidden.showTime).toBe(false);
    expect(setDetailColumnWidth(hidden, "type", 2).typeWidth).toBe(64);
    expect(setDetailColumnWidth(hidden, "type", 900).typeWidth).toBe(600);
    expect(
      resetDetailColumnWidths({ ...hidden, typeWidth: 500 }),
    ).toMatchObject({ showTime: false, typeWidth: 250 });
  });

  it("virtualizes a bounded overscanned row range", () => {
    expect(detailVisibleRowRange(1_000, 640, 320, 2)).toEqual({
      start: 8,
      end: 17,
    });
    expect(detailVisibleRowRange(5, 64_000, 320, 2)).toEqual({
      start: 2,
      end: 5,
    });
  });

  it("keeps already paged rows when a live snapshot refreshes the first page", () => {
    const current = fakeSnapshot(["a", "b", "c"], 4);
    const next = fakeSnapshot(["a", "b"], 4);
    expect(
      mergeLiveDetailSnapshot(current, next).rows.map((row) => row.id),
    ).toEqual(["a", "b", "c"]);
  });

  it("drops a stale paged tail when the refreshed page boundary changes", () => {
    const current = fakeSnapshot(["a", "b", "c"], 4);
    const next = fakeSnapshot(["a2", "b2"], 4);
    expect(
      mergeLiveDetailSnapshot(current, next).rows.map((row) => row.id),
    ).toEqual(["a2", "b2"]);
  });

  it("deduplicates rows when a live stream advances before loadMore resolves", () => {
    const current = fakeSnapshot(["a", "b", "c", "d"], 6);
    const next = fakeSnapshot(["c", "d", "e"], 6);
    expect(
      mergePagedDetailSnapshot(current, next).rows.map((row) => row.id),
    ).toEqual(["a", "b", "c", "d", "e"]);
  });

  it("replaces the view when loadMore resolves into a different detail view", () => {
    const current = fakeSnapshot(["a", "b"], 2);
    const next = { ...fakeSnapshot(["x"], 1), kind: "character" as const };
    expect(
      mergePagedDetailSnapshot(current, next).rows.map((row) => row.id),
    ).toEqual(["x"]);
  });
});

function fakeSnapshot(ids: string[], totalHits: number): MainDpsDetailSnapshot {
  return {
    contractVersion: 4,
    generation: "1",
    kind: "team",
    abyssHalf: null,
    characterId: null,
    characterName: null,
    characterColor: null,
    filter: "all",
    qteType: null,
    skillFilter: null,
    columns: DEFAULT_DETAIL_COLUMNS,
    actions: { canStartCapture: true, canImportReplay: true },
    metrics: {
      totalOutput: 0,
      dps: 0,
      outputCount: 0,
      incomingCount: 0,
      totalDamageTaken: 0,
      durationSeconds: 0,
    },
    direction: {
      confirmedOutput: 0,
      confirmedHits: 0,
      candidateOutput: 0,
      candidateHits: 0,
      incomingOutput: 0,
      incomingHits: 0,
      candidateSharePercent: 0,
    },
    hitTypes: [],
    attribution: {
      totalDamage: 0,
      characterDamage: 0,
      characterFilter: "characterAttributed",
      reactionDamage: 0,
      sharedDamage: 0,
      unattributedDamage: 0,
      separateReactionDamage: false,
    },
    qteSummaries: [],
    qteSummaryTotalCount: 0,
    qteSummariesTruncated: false,
    skills: [],
    skillTotalCount: 0,
    skillsTruncated: false,
    textTruncated: false,
    totalHits,
    totalDamage: 0,
    maxRowDamage: 1,
    offset: 0,
    rows: ids.map((id) => ({
      id,
      timestamp: 0,
      characterId: 0,
      characterName: "-",
      direction: "outgoing",
      damage: 0,
      primaryDamage: 0,
      followUpDamage: 0,
      skillId: "-",
      skill: "-",
      damageType: "-",
      typeLabel: "-",
      reactionTextKey: null,
      damageDigitKey: null,
      followUpDamageDigitKey: null,
      target: "-",
      targetMonsterId: null,
      targetHpAfter: 0,
      targetMaxHp: 0,
      targetHpPercent: 0,
    })),
  };
}
