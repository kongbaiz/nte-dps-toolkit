import {
  historyError,
  parseHistoryFileActionResult,
  parseHistoryImportFileResult,
  parseHistoryDeleteResult,
  parseHistoryEvent,
  parseHistoryComparison,
  parseHistoryExport,
  parseHistorySnapshot,
  type HistoryCommandError,
  type HistoryComparison,
  type HistoryDeleteResult,
  type HistoryExport,
  type HistoryFileActionResult,
  type HistoryImportFileResult,
  type HistoryLine,
  type HistorySnapshot,
} from "@/lib/tauri/history-contract";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";
import {
  subscribeStream,
  tauriStreamTransport,
} from "@/lib/tauri/stream-client";

const COMMANDS = {
  compare: "compare_history_records",
  delete: "delete_history_record",
  export: "export_history_record_json",
  exportFile: "export_history_record_file",
  getSnapshot: "get_history_snapshot",
  import: "import_history_record_json",
  importFile: "import_history_record_file",
  save: "save_current_history_summary",
  restore: "restore_deleted_history_record",
  setPrediction: "set_history_prediction_team",
  subscribe: "subscribe_history",
  unsubscribe: "unsubscribe_history",
} as const;

interface HistoryTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  createChannel(onMessage: (message: unknown) => void): unknown;
}

export interface HistoryClient {
  getSnapshot(): Promise<HistorySnapshot>;
  saveCurrent(): Promise<HistorySnapshot>;
  importJson(json: string): Promise<HistorySnapshot>;
  importFile(): Promise<HistoryImportFileResult>;
  deleteRecord(recordId: string): Promise<HistoryDeleteResult>;
  restoreDeleted(undoToken: string): Promise<HistorySnapshot>;
  exportRecord(recordId: string): Promise<HistoryExport>;
  exportRecordFile(recordId: string): Promise<HistoryFileActionResult>;
  compare(leftId: string, rightId: string): Promise<HistoryComparison>;
  setPrediction(recordId: string, line: HistoryLine): Promise<HistorySnapshot>;
  subscribe(
    onSnapshot: (snapshot: HistorySnapshot) => void,
    onError: (error: HistoryCommandError) => void,
  ): () => Promise<void>;
}

const tauriTransport: HistoryTransport = tauriStreamTransport;

export function createHistoryClient(
  transport: HistoryTransport = tauriTransport,
  createSubscriptionId: () => string = () => crypto.randomUUID(),
): HistoryClient {
  async function run<T>(
    command: string,
    parser: (value: unknown) => T,
    arguments_?: Record<string, unknown>,
  ): Promise<T> {
    try {
      return parser(await transport.invoke(command, arguments_));
    } catch (error) {
      if (error instanceof TechnicalContractError) throw error;
      throw historyError(error);
    }
  }
  const snapshot = (command: string, arguments_?: Record<string, unknown>) =>
    run(command, parseHistorySnapshot, arguments_);
  return {
    getSnapshot: () => snapshot(COMMANDS.getSnapshot),
    saveCurrent: () => snapshot(COMMANDS.save),
    importJson: (json) => snapshot(COMMANDS.import, { json }),
    importFile: () => run(COMMANDS.importFile, parseHistoryImportFileResult),
    deleteRecord: (recordId) =>
      run(COMMANDS.delete, parseHistoryDeleteResult, { recordId }),
    restoreDeleted: (undoToken) => snapshot(COMMANDS.restore, { undoToken }),
    exportRecord: (recordId) =>
      run(COMMANDS.export, parseHistoryExport, { recordId }),
    exportRecordFile: (recordId) =>
      run(COMMANDS.exportFile, parseHistoryFileActionResult, { recordId }),
    compare: (leftId, rightId) =>
      run(COMMANDS.compare, parseHistoryComparison, { leftId, rightId }),
    setPrediction: (recordId, line) =>
      snapshot(COMMANDS.setPrediction, { recordId, line }),
    subscribe: (onSnapshot, onError) => {
      const subscriptionId = createSubscriptionId();
      return subscribeStream({
        transport,
        streamKind: "history",
        subscriptionId,
        subscribeCommand: COMMANDS.subscribe,
        unsubscribeCommand: COMMANDS.unsubscribe,
        parseEvent: parseHistoryEvent,
        onEvent: onSnapshot,
        onError: (error) => onError(historyError(error)),
      });
    },
  };
}

export function historyClientError(error: unknown): HistoryCommandError {
  return historyError(error);
}

export const historyClient = createHistoryClient();
