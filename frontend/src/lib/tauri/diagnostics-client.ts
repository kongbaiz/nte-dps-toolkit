import {
  diagnosticsError,
  parseDiagnosticsActionResult,
  parseDiagnosticsEvent,
  parseDiagnosticsSnapshot,
  type DiagnosticsActionResult,
  type DiagnosticsCommandError,
  type DiagnosticsSnapshot,
} from "@/lib/tauri/diagnostics-contract";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";
import {
  subscribeStream,
  tauriStreamTransport,
  type StreamTransport,
} from "@/lib/tauri/stream-client";

const COMMANDS = {
  getSnapshot: "get_diagnostics_snapshot",
  run: "run_diagnostics",
  importPcapng: "import_diagnostics_pcapng",
  importJson: "import_diagnostics_json",
  exportJson: "export_diagnostics_json",
  exportPcapng: "export_diagnostics_pcapng",
  subscribe: "subscribe_diagnostics",
  unsubscribe: "unsubscribe_diagnostics",
} as const;

export interface DiagnosticsClient {
  getSnapshot(): Promise<DiagnosticsSnapshot>;
  run(): Promise<DiagnosticsSnapshot>;
  importPcapng(): Promise<DiagnosticsActionResult>;
  importJson(): Promise<DiagnosticsActionResult>;
  exportJson(): Promise<DiagnosticsActionResult>;
  exportPcapng(): Promise<DiagnosticsActionResult>;
  subscribe(
    onSnapshot: (snapshot: DiagnosticsSnapshot) => void,
    onError: (error: DiagnosticsCommandError) => void,
  ): () => Promise<void>;
}

export function createDiagnosticsClient(
  transport: StreamTransport = tauriStreamTransport,
  createSubscriptionId: () => string = () => crypto.randomUUID(),
): DiagnosticsClient {
  const invokeSnapshot = async (command: string) => {
    try {
      return parseDiagnosticsSnapshot(await transport.invoke(command));
    } catch (error) {
      if (error instanceof TechnicalContractError) throw error;
      throw diagnosticsError(error);
    }
  };
  const invokeAction = async (command: string) => {
    try {
      return parseDiagnosticsActionResult(await transport.invoke(command));
    } catch (error) {
      if (error instanceof TechnicalContractError) throw error;
      throw diagnosticsError(error);
    }
  };

  return {
    getSnapshot: () => invokeSnapshot(COMMANDS.getSnapshot),
    run: () => invokeSnapshot(COMMANDS.run),
    importPcapng: () => invokeAction(COMMANDS.importPcapng),
    importJson: () => invokeAction(COMMANDS.importJson),
    exportJson: () => invokeAction(COMMANDS.exportJson),
    exportPcapng: () => invokeAction(COMMANDS.exportPcapng),
    subscribe: (onSnapshot, onError) => {
      const subscriptionId = createSubscriptionId();
      return subscribeStream({
        transport,
        streamKind: "diagnostics",
        subscriptionId,
        subscribeCommand: COMMANDS.subscribe,
        unsubscribeCommand: COMMANDS.unsubscribe,
        parseEvent: parseDiagnosticsEvent,
        onEvent: onSnapshot,
        onError: (error) => onError(diagnosticsError(error)),
      });
    },
  };
}

export const diagnosticsClient = createDiagnosticsClient();
