import { invokeCommand, type CommandError } from "@/lib/tauri/command-error";
import type { DiagnosticsActionResult } from "@/lib/tauri/generated/DiagnosticsActionResult";
import type { DiagnosticsEvent } from "@/lib/tauri/generated/DiagnosticsEvent";
import type { DiagnosticsSnapshot } from "@/lib/tauri/generated/DiagnosticsSnapshot";
import { subscribeStream } from "@/lib/tauri/stream-client";

const action = (command: string) => () =>
  invokeCommand<DiagnosticsActionResult>(command);

export const diagnosticsClient = {
  getSnapshot: () =>
    invokeCommand<DiagnosticsSnapshot>("get_diagnostics_snapshot"),
  run: () => invokeCommand<DiagnosticsSnapshot>("run_diagnostics"),
  importPcapng: action("import_diagnostics_pcapng"),
  importJson: action("import_diagnostics_json"),
  exportJson: action("export_diagnostics_json"),
  exportPcapng: action("export_diagnostics_pcapng"),
  subscribe: (
    onSnapshot: (snapshot: DiagnosticsSnapshot) => void,
    onError: (error: CommandError) => void,
  ) =>
    subscribeStream<DiagnosticsEvent>({
      streamKind: "diagnostics",
      subscribeCommand: "subscribe_diagnostics",
      unsubscribeCommand: "unsubscribe_diagnostics",
      onEvent: (event) => onSnapshot(event.payload),
      onError,
    }),
};
