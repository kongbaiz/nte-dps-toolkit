import { Channel, invoke } from "@tauri-apps/api/core";

import {
  historyError,
  parseHistoryDeleteResult,
  parseHistoryEvent,
  parseHistoryComparison,
  parseHistoryExport,
  parseHistorySnapshot,
  type HistoryCommandError,
  type HistoryComparison,
  type HistoryDeleteResult,
  type HistoryExport,
  type HistoryLine,
  type HistorySnapshot,
} from "@/lib/tauri/history-contract";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";
import { parseSubscriptionReceipt } from "@/lib/tauri/technical-contract";

const COMMANDS = {
  compare: "compare_history_records",
  delete: "delete_history_record",
  export: "export_history_record_json",
  getSnapshot: "get_history_snapshot",
  import: "import_history_record_json",
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
  deleteRecord(recordId: string): Promise<HistoryDeleteResult>;
  restoreDeleted(undoToken: string): Promise<HistorySnapshot>;
  exportRecord(recordId: string): Promise<HistoryExport>;
  compare(leftId: string, rightId: string): Promise<HistoryComparison>;
  setPrediction(recordId: string, line: HistoryLine): Promise<HistorySnapshot>;
  subscribe(
    onSnapshot: (snapshot: HistorySnapshot) => void,
    onError: (error: HistoryCommandError) => void,
  ): () => Promise<void>;
}

const tauriTransport: HistoryTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
  createChannel: (onMessage) => {
    const channel = new Channel<unknown>();
    channel.onmessage = onMessage;
    return channel;
  },
};

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
    deleteRecord: (recordId) =>
      run(COMMANDS.delete, parseHistoryDeleteResult, { recordId }),
    restoreDeleted: (undoToken) => snapshot(COMMANDS.restore, { undoToken }),
    exportRecord: (recordId) =>
      run(COMMANDS.export, parseHistoryExport, { recordId }),
    compare: (leftId, rightId) =>
      run(COMMANDS.compare, parseHistoryComparison, { leftId, rightId }),
    setPrediction: (recordId, line) =>
      snapshot(COMMANDS.setPrediction, { recordId, line }),
    subscribe: (onSnapshot, onError) => {
      const subscriptionId = createSubscriptionId();
      let closed = false;
      const onEvent = transport.createChannel((message) => {
        if (closed) return;
        try {
          onSnapshot(parseHistoryEvent(message));
        } catch (error) {
          onError(historyError(error));
        }
      });
      const receipt = transport
        .invoke(COMMANDS.subscribe, { subscriptionId, onEvent })
        .then(parseSubscriptionReceipt)
        .catch((error: unknown) => {
          if (!closed) onError(historyError(error));
          return undefined;
        });
      return async () => {
        if (closed) return;
        closed = true;
        const activeReceipt = await receipt;
        if (activeReceipt) {
          await transport.invoke(COMMANDS.unsubscribe, {
            subscriptionId: activeReceipt.subscriptionId,
          });
        }
      };
    },
  };
}

export function historyClientError(error: unknown): HistoryCommandError {
  return historyError(error);
}

export const historyClient = createHistoryClient();
