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

export function validHistoryComparisonPair(
  records: HistoryRecord[],
  leftId: string,
  rightId: string,
): boolean {
  return (
    leftId !== rightId &&
    records.some((record) => record.id === leftId) &&
    records.some((record) => record.id === rightId)
  );
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

export function historyDurationFormat(seconds: number): {
  key: "{}m{}s" | "{}s";
  arguments: string[];
} {
  if (seconds >= 60) {
    const minutes = Math.floor(seconds / 60);
    const remainder = seconds - minutes * 60;
    return {
      key: "{}m{}s",
      arguments: [String(minutes), remainder.toFixed(1).padStart(4, "0")],
    };
  }
  return { key: "{}s", arguments: [seconds.toFixed(1)] };
}

export function historyComparisonWarningKeys(comparison: {
  differentTimeBasis: boolean;
  differentReactionAccounting: boolean;
}): string[] {
  const warnings: string[] = [];
  if (comparison.differentTimeBasis) {
    warnings.push(
      "The two records use different DPS time bases; compare with care",
    );
  }
  if (comparison.differentReactionAccounting) {
    warnings.push(
      "The two records use different reaction damage accounting; compare with care",
    );
  }
  return warnings;
}

export function historyDeltaTone(
  value: number,
): "positive" | "negative" | "neutral" {
  if (value > 0) return "positive";
  if (value < 0) return "negative";
  return "neutral";
}
