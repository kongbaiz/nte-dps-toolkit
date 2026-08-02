import { describe, expect, it } from "vitest";

import type { HistoryRecord } from "@/lib/tauri/history-contract";
import {
  adjacentHistoryRecordId,
  historyComparisonWarningKeys,
  historyDeltaTone,
  historyDurationFormat,
  nextHistoryRecordIndex,
  selectHistoryRecord,
} from "./history-view-model";

const record = (id: string) => ({ id }) as HistoryRecord;

describe("History view model", () => {
  it("preserves selection across refresh and falls back to newest", () => {
    const records = [record("new"), record("old")];
    expect(selectHistoryRecord(records, "old")).toBe("old");
    expect(selectHistoryRecord(records, "missing")).toBe("new");
    expect(selectHistoryRecord([], "old")).toBeNull();
  });

  it("chooses an adjacent comparison record on both ends", () => {
    const records = [record("a"), record("b"), record("c")];
    expect(adjacentHistoryRecordId(records, "a")).toBe("b");
    expect(adjacentHistoryRecordId(records, "c")).toBe("b");
    expect(adjacentHistoryRecordId([record("a")], "a")).toBeNull();
  });

  it("keeps keyboard selection inside the record list", () => {
    expect(nextHistoryRecordIndex(3, 0, -1)).toBe(0);
    expect(nextHistoryRecordIndex(3, 1, 1)).toBe(2);
    expect(nextHistoryRecordIndex(3, 2, 1)).toBe(2);
  });

  it("formats combat duration with the same minute boundary as egui", () => {
    expect(historyDurationFormat(9.25)).toEqual({
      key: "{}s",
      arguments: ["9.3"],
    });
    expect(historyDurationFormat(65.25)).toEqual({
      key: "{}m{}s",
      arguments: ["1", "05.3"],
    });
  });

  it("keeps the two comparison compatibility warnings distinct", () => {
    expect(
      historyComparisonWarningKeys({
        differentTimeBasis: true,
        differentReactionAccounting: true,
      }),
    ).toEqual([
      "The two records use different DPS time bases; compare with care",
      "The two records use different reaction damage accounting; compare with care",
    ]);
  });

  it("uses a neutral tone for a zero comparison delta", () => {
    expect(historyDeltaTone(1)).toBe("positive");
    expect(historyDeltaTone(-1)).toBe("negative");
    expect(historyDeltaTone(0)).toBe("neutral");
  });
});
