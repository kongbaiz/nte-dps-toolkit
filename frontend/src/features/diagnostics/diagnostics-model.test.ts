import { describe, expect, it } from "vitest";

import type { DiagnosticsSnapshot } from "@/lib/tauri/diagnostics-contract";
import { formatByteCount } from "@/lib/update-presentation";

import {
  buildRedactedDiagnosticsReport,
  diagnosticsContentKind,
  diagnosticsSnapshotIsAtLeast,
  formatDecimalString,
} from "./diagnostics-model";

const SNAPSHOT = {
  capture: { phase: "idle", droppedHistoryArchives: "0" },
  quality: { source: "unknown", packetCount: 3, hitCount: 2 },
  report: {
    failedCount: 1,
    warningCount: 0,
    checks: [
      {
        status: "failed",
        titleKey: "Npcap",
        detail: { messageKey: "private {}", messageArguments: ["path"] },
        suggestion: {
          messageKey: "Install {}",
          messageArguments: ["Npcap"],
        },
      },
    ],
  },
} as DiagnosticsSnapshot;

describe("Diagnostics model", () => {
  it("formats bounded numeric projections", () => {
    expect(formatDecimalString("12345678901234567890")).toContain(",");
    expect(formatByteCount("1536")).toBe("1.5 KiB");
    expect(diagnosticsContentKind("ready")).toBe("ready");
  });

  it("rejects a query response older than a streamed snapshot", () => {
    const current = {
      ...SNAPSHOT,
      captureGeneration: "8",
      qualityGeneration: "9",
      reportGeneration: "3",
    };
    expect(
      diagnosticsSnapshotIsAtLeast(
        { ...current, qualityGeneration: "8" },
        current,
      ),
    ).toBe(false);
    expect(
      diagnosticsSnapshotIsAtLeast(
        { ...current, reportGeneration: "4" },
        current,
      ),
    ).toBe(true);
  });

  it("copies a redacted report without check details", () => {
    const text = buildRedactedDiagnosticsReport(
      SNAPSHOT,
      (key) => key,
      (key, arguments_) => key.replace("{}", arguments_[0] ?? ""),
    );
    expect(text).toContain("Install Npcap");
    expect(text).not.toContain("private path");
  });
});
