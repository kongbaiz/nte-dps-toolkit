import { invokeCommand, type CommandError } from "@/lib/tauri/command-error";
import type { AbyssHalf } from "@/lib/tauri/generated/AbyssHalf";
import type { HistoryComparisonSnapshot } from "@/lib/tauri/generated/HistoryComparisonSnapshot";
import type { HistoryDeleteSnapshot } from "@/lib/tauri/generated/HistoryDeleteSnapshot";
import type { HistoryEvent } from "@/lib/tauri/generated/HistoryEvent";
import type { HistoryExportSnapshot } from "@/lib/tauri/generated/HistoryExportSnapshot";
import type { HistoryFileActionSnapshot } from "@/lib/tauri/generated/HistoryFileActionSnapshot";
import type { HistoryImportFileSnapshot } from "@/lib/tauri/generated/HistoryImportFileSnapshot";
import type { HistorySnapshot } from "@/lib/tauri/generated/HistorySnapshot";
import { subscribeStream } from "@/lib/tauri/stream-client";

export const historyClient = {
  getSnapshot: () => invokeCommand<HistorySnapshot>("get_history_snapshot"),
  saveCurrent: () =>
    invokeCommand<HistorySnapshot>("save_current_history_summary"),
  importJson: (json: string) =>
    invokeCommand<HistorySnapshot>("import_history_record_json", { json }),
  importFile: () =>
    invokeCommand<HistoryImportFileSnapshot>("import_history_record_file"),
  deleteRecord: (recordId: string) =>
    invokeCommand<HistoryDeleteSnapshot>("delete_history_record", {
      recordId,
    }),
  restoreDeleted: (undoToken: string) =>
    invokeCommand<HistorySnapshot>("restore_deleted_history_record", {
      undoToken,
    }),
  exportRecord: (recordId: string) =>
    invokeCommand<HistoryExportSnapshot>("export_history_record_json", {
      recordId,
    }),
  exportRecordFile: (recordId: string) =>
    invokeCommand<HistoryFileActionSnapshot>("export_history_record_file", {
      recordId,
    }),
  compare: (leftId: string, rightId: string) =>
    invokeCommand<HistoryComparisonSnapshot>("compare_history_records", {
      leftId,
      rightId,
    }),
  setPrediction: (recordId: string, line: AbyssHalf) =>
    invokeCommand<HistorySnapshot>("set_history_prediction_team", {
      recordId,
      line,
    }),
  subscribe: (
    onSnapshot: (snapshot: HistorySnapshot) => void,
    onError: (error: CommandError) => void,
  ) =>
    subscribeStream<HistoryEvent>({
      streamKind: "history",
      subscribeCommand: "subscribe_history",
      unsubscribeCommand: "unsubscribe_history",
      onEvent: (event) => onSnapshot(event.payload),
      onError,
    }),
};
