import type {
  DiagnosticsReportSnapshot,
  DiagnosticsSnapshot,
} from "@/lib/tauri/diagnostics-contract";

export type DiagnosticsContentKind = "loading" | "error" | "ready";

export function diagnosticsContentKind(
  status: "loading" | "error" | "ready",
): DiagnosticsContentKind {
  return status;
}

export function diagnosticsSnapshotIsAtLeast(
  next: DiagnosticsSnapshot,
  current: DiagnosticsSnapshot | null,
): boolean {
  if (current === null) return true;
  return (
    BigInt(next.captureGeneration) >= BigInt(current.captureGeneration) &&
    BigInt(next.qualityGeneration) >= BigInt(current.qualityGeneration) &&
    BigInt(next.reportGeneration) >= BigInt(current.reportGeneration)
  );
}

export function formatDecimalString(value: string): string {
  try {
    return BigInt(value).toLocaleString();
  } catch {
    return value;
  }
}

export function formatByteCount(value: string): string {
  const numeric = Number(value);
  if (!Number.isFinite(numeric) || numeric < 0) return value;
  if (numeric < 1_024) return `${numeric.toLocaleString()} B`;
  if (numeric < 1_048_576) return `${(numeric / 1_024).toFixed(1)} KiB`;
  if (numeric < 1_073_741_824) {
    return `${(numeric / 1_048_576).toFixed(1)} MiB`;
  }
  return `${(numeric / 1_073_741_824).toFixed(1)} GiB`;
}

export function buildRedactedDiagnosticsReport(
  snapshot: DiagnosticsSnapshot,
  translate: (key: string) => string,
  formatMessage: (key: string, arguments_: readonly string[]) => string,
): string {
  const lines = [
    "NTE Diagnostics",
    `${translate("Capture status")}: ${translate(snapshot.capture.phase)}`,
    `${translate("Capture quality source")}: ${translate(snapshot.quality.source)}`,
    `${translate("Packets / hits")}: ${snapshot.quality.packetCount.toLocaleString()} / ${snapshot.quality.hitCount.toLocaleString()}`,
  ];
  const report: DiagnosticsReportSnapshot | null = snapshot.report;
  if (report) {
    lines.push(
      `${translate("Failed / warnings")}: ${report.failedCount.toLocaleString()} / ${report.warningCount.toLocaleString()}`,
    );
    for (const check of report.checks) {
      lines.push(
        `[${translate(check.status)}] ${translate(check.titleKey)}: ${formatMessage(check.suggestion.messageKey, check.suggestion.messageArguments)}`,
      );
    }
  }
  return lines.join("\n");
}
