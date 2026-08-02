import type {
  HistoryRecord,
  HistorySnapshot,
} from "@/lib/tauri/history-contract";

export type HistoryPageState =
  | { status: "loading" }
  | { status: "error"; messageKey: string }
  | { status: "ready"; snapshot: HistorySnapshot };

export function selectHistoryRecord(
  records: HistoryRecord[],
  selectedId: string | null,
): string | null {
  if (
    selectedId !== null &&
    records.some((record) => record.id === selectedId)
  ) {
    return selectedId;
  }
  return records[0]?.id ?? null;
}

export function adjacentHistoryRecordId(
  records: HistoryRecord[],
  recordId: string,
): string | null {
  const index = records.findIndex((record) => record.id === recordId);
  if (index < 0 || records.length < 2) return null;
  return records[index + 1]?.id ?? records[index - 1]?.id ?? null;
}

export function historyRecordById(
  records: HistoryRecord[],
  recordId: string | null,
): HistoryRecord | null {
  return records.find((record) => record.id === recordId) ?? null;
}

export function nextHistoryRecordIndex(
  length: number,
  current: number,
  direction: -1 | 1,
): number {
  return Math.max(0, Math.min(length - 1, current + direction));
}
