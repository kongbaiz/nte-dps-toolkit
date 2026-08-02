import { describe, expect, it } from "vitest";

import { createHistoryClient } from "./history-client";

describe("History client", () => {
  it("routes comparison through the typed Tauri command", async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = [];
    const client = createHistoryClient({
      async invoke(command, arguments_) {
        calls.push([command, arguments_]);
        return {
          leftId: "left",
          rightId: "right",
          totalDpsDelta: 1,
          totalDamageDelta: 2,
          durationDelta: 3,
          differentTimeBasis: false,
          differentReactionAccounting: false,
          characterDeltas: [],
          skillDeltas: [],
        };
      },
      createChannel() {
        return {};
      },
    });

    const result = await client.compare("left", "right");

    expect(result.totalDamageDelta).toBe(2);
    expect(calls).toEqual([
      ["compare_history_records", { leftId: "left", rightId: "right" }],
    ]);
  });

  it("routes history import and export through native file commands", async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = [];
    const client = createHistoryClient({
      async invoke(command, arguments_) {
        calls.push([command, arguments_]);
        if (command === "import_history_record_file") {
          return {
            performed: true,
            importedRecordId: "record-1",
            history: {
              contractVersion: 2,
              revision: "1",
              maxImportBytes: "134217728",
              skippedFiles: 0,
              records: [],
            },
          };
        }
        return { performed: true };
      },
      createChannel() {
        return {};
      },
    });

    expect((await client.importFile()).importedRecordId).toBe("record-1");
    expect(await client.exportRecordFile("record-1")).toEqual({
      performed: true,
    });
    expect(calls).toEqual([
      ["import_history_record_file", undefined],
      ["export_history_record_file", { recordId: "record-1" }],
    ]);
  });

  it("subscribes through a typed Channel and cleans it up", async () => {
    const calls: string[] = [];
    let onMessage: ((message: unknown) => void) | undefined;
    const client = createHistoryClient(
      {
        async invoke(command) {
          calls.push(command);
          if (command === "subscribe_history") {
            return { subscriptionId: "history-test", streamIntervalMs: 250 };
          }
          return undefined;
        },
        createChannel(handler) {
          onMessage = handler;
          return {};
        },
      },
      () => "history-test",
    );
    const snapshots: unknown[] = [];
    const unsubscribe = client.subscribe(
      (snapshot) => snapshots.push(snapshot),
      (error) => {
        throw error;
      },
    );
    onMessage?.({
      event: "snapshot",
      payload: {
        contractVersion: 2,
        revision: "1",
        maxImportBytes: "1",
        skippedFiles: 0,
        records: [],
      },
    });
    await unsubscribe();
    expect(snapshots).toHaveLength(1);
    expect(calls).toEqual(["subscribe_history", "unsubscribe_history"]);
  });
});
