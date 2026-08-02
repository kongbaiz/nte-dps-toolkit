import { Channel, invoke } from "@tauri-apps/api/core";

import {
  diagnosticsError,
  parseDiagnosticsActionResult,
  parseDiagnosticsEvent,
  parseDiagnosticsSnapshot,
  type DiagnosticsActionResult,
  type DiagnosticsCommandError,
  type DiagnosticsSnapshot,
} from "@/lib/tauri/diagnostics-contract";
import {
  parseSubscriptionReceipt,
  TechnicalContractError,
} from "@/lib/tauri/technical-contract";

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

interface DiagnosticsTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  createChannel(onMessage: (message: unknown) => void): unknown;
}

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

const tauriTransport: DiagnosticsTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
  createChannel: (onMessage) => {
    const channel = new Channel<unknown>();
    channel.onmessage = onMessage;
    return channel;
  },
};

export function createDiagnosticsClient(
  transport: DiagnosticsTransport = tauriTransport,
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
      let closed = false;
      const onEvent = transport.createChannel((message) => {
        if (closed) return;
        try {
          onSnapshot(parseDiagnosticsEvent(message));
        } catch (error) {
          onError(diagnosticsError(error));
        }
      });
      const receipt = transport
        .invoke(COMMANDS.subscribe, { subscriptionId, onEvent })
        .then(parseSubscriptionReceipt)
        .catch((error: unknown) => {
          if (!closed) onError(diagnosticsError(error));
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

export const diagnosticsClient = createDiagnosticsClient();
