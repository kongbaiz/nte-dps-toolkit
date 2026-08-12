import { describe, expect, it } from "vitest";

import {
  DIAGNOSTICS_MAX_CHECKS,
  parseDiagnosticsActionResult,
  parseDiagnosticsEvent,
  parseDiagnosticsSnapshot,
} from "@/lib/tauri/diagnostics-contract";

const SNAPSHOT = {
  contractVersion: 2,
  captureGeneration: "9007199254740993",
  qualityGeneration: "9007199254740994",
  reportGeneration: "2",
  adapterVersion: "0.3.6",
  capture: {
    phase: "stopped",
    replayRunning: false,
    activeFilter: "udp",
    droppedHistoryArchives: "0",
    rawCapture: {
      fileName: "nte_raw.pcapng",
      packetCount: "9007199254740994",
      capturedBytes: "128",
      writeError: false,
      writing: false,
    },
  },
  environment: {
    deviceLabel: "NIC",
    manualDevice: false,
    localIp: "192.0.2.1",
    gameConnection: {
      pid: 7,
      localIp: "192.0.2.1",
      remoteIp: "198.51.100.2",
      remotePort: 30196,
    },
  },
  report: {
    failedCount: 0,
    warningCount: 1,
    checks: [
      {
        status: "warning",
        titleKey: "Capture Status",
        detail: {
          messageKey: "No live capture task right now",
          messageArguments: [],
        },
        suggestion: {
          messageKey:
            "Run diagnostics after clicking Start to see BPF and raw-capture write status",
          messageArguments: [],
        },
      },
    ],
  },
  quality: {
    source: "live",
    packetCount: 4,
    packetsWithHits: 2,
    hitCount: 3,
    outgoingHits: "2",
    outgoingDamage: 100,
    unknownDirectionHits: "0",
    unknownDirectionDamage: 0,
    incomingHits: "1",
    incomingDamage: 5,
    unknownCharacterCount: 0,
    unknownCharacterHits: "0",
    unmappedSkillRows: 0,
    unmappedSkillHits: "0",
    unmappedGameplayEffectCount: 0,
    timeStopEventCount: "1",
    timeStopIntervalCount: 1,
    abyssEventCount: "0",
    serverDamageCorrections: "0",
  },
  actions: {
    canImport: true,
    canExportParsed: true,
    canExportRaw: true,
  },
};

describe("Diagnostics contract", () => {
  it("keeps generations and 64-bit counters as decimal strings", () => {
    const snapshot = parseDiagnosticsSnapshot(SNAPSHOT);

    expect(snapshot.captureGeneration).toBe("9007199254740993");
    expect(snapshot.qualityGeneration).toBe("9007199254740994");
    expect(snapshot.capture.rawCapture?.packetCount).toBe("9007199254740994");
  });

  it("parses tagged events and action results", () => {
    expect(
      parseDiagnosticsEvent({ event: "snapshot", payload: SNAPSHOT }).report
        ?.warningCount,
    ).toBe(1);
    expect(
      parseDiagnosticsActionResult({ performed: true, snapshot: SNAPSHOT })
        .performed,
    ).toBe(true);
  });

  it("rejects unknown statuses and oversized reports", () => {
    expect(() =>
      parseDiagnosticsSnapshot({
        ...SNAPSHOT,
        report: {
          ...SNAPSHOT.report,
          checks: [{ ...SNAPSHOT.report.checks[0], status: "private-status" }],
        },
      }),
    ).toThrow(/status/);
    expect(() =>
      parseDiagnosticsSnapshot({
        ...SNAPSHOT,
        report: {
          ...SNAPSHOT.report,
          checks: Array.from(
            { length: DIAGNOSTICS_MAX_CHECKS + 1 },
            () => SNAPSHOT.report.checks[0],
          ),
        },
      }),
    ).toThrow(/bounds/);
  });
});
