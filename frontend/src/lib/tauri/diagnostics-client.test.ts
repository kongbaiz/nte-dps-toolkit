import { describe, expect, it, vi } from "vitest";

import { createDiagnosticsClient } from "@/lib/tauri/diagnostics-client";

const SNAPSHOT = {
  contractVersion: 2,
  captureGeneration: "4",
  qualityGeneration: "5",
  reportGeneration: "2",
  adapterVersion: "Npcap 1.80",
  capture: {
    phase: "idle",
    replayRunning: false,
    activeFilter: null,
    droppedHistoryArchives: "0",
    rawCapture: null,
  },
  environment: null,
  report: null,
  quality: {
    source: "unknown",
    packetCount: 0,
    packetsWithHits: 0,
    hitCount: 0,
    outgoingHits: "0",
    outgoingDamage: 0,
    unknownDirectionHits: "0",
    unknownDirectionDamage: 0,
    incomingHits: "0",
    incomingDamage: 0,
    unknownCharacterCount: 0,
    unknownCharacterHits: "0",
    unmappedSkillRows: 0,
    unmappedSkillHits: "0",
    unmappedGameplayEffectCount: 0,
    timeStopEventCount: "0",
    timeStopIntervalCount: 0,
    abyssEventCount: "0",
    serverDamageCorrections: "0",
  },
  actions: {
    canImport: true,
    canExportParsed: false,
    canExportRaw: false,
  },
};

describe("Diagnostics client", () => {
  it("uses typed action commands", async () => {
    const invoke = vi.fn(async (command: string) => {
      if (command === "run_diagnostics") return SNAPSHOT;
      return { performed: true, snapshot: SNAPSHOT };
    });
    const client = createDiagnosticsClient({
      invoke,
      createChannel: vi.fn(),
    });

    expect((await client.run()).captureGeneration).toBe("4");
    expect((await client.importPcapng()).performed).toBe(true);
    expect((await client.importJson()).performed).toBe(true);
    expect((await client.exportJson()).performed).toBe(true);
    expect((await client.exportPcapng()).performed).toBe(true);
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
      "run_diagnostics",
      "import_diagnostics_pcapng",
      "import_diagnostics_json",
      "export_diagnostics_json",
      "export_diagnostics_pcapng",
    ]);
  });

  it("releases the acknowledged diagnostics stream", async () => {
    let onMessage: ((message: unknown) => void) | undefined;
    const invoke = vi.fn(async (command: string) => {
      if (command === "subscribe_diagnostics") {
        return { subscriptionId: "diagnostics-test", streamIntervalMs: 500 };
      }
      return undefined;
    });
    const client = createDiagnosticsClient(
      {
        invoke,
        createChannel: (handler) => {
          onMessage = handler;
          return { channel: true };
        },
      },
      () => "diagnostics-test",
    );
    const received = vi.fn();
    const unsubscribe = client.subscribe(received, vi.fn());
    onMessage?.({ event: "snapshot", payload: SNAPSHOT });
    await unsubscribe();

    expect(received).toHaveBeenCalledWith(
      expect.objectContaining({ captureGeneration: "4" }),
    );
    expect(invoke).toHaveBeenCalledWith(
      "subscribe_diagnostics",
      expect.objectContaining({ subscriptionId: "diagnostics-test" }),
    );
    expect(invoke).toHaveBeenCalledWith("unsubscribe_diagnostics", {
      subscriptionId: "diagnostics-test",
    });
  });
});
