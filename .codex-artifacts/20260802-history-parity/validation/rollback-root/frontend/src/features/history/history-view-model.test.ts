import { describe, expect, it } from "vitest";

import type { HistoryRecord } from "@/lib/tauri/history-contract";
import {
  adjacentHistoryRecordId,
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
});
