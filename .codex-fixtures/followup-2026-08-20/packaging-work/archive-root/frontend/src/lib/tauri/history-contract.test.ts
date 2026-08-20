import { describe, expect, it } from "vitest";

import {
  HISTORY_MAX_INLINE_EXPORT_CHARACTERS,
  parseHistoryExport,
  parseHistoryFileActionResult,
  parseHistoryImportFileResult,
  parseHistorySnapshot,
} from "./history-contract";

function snapshot() {
  return {
    contractVersion: 2,
    revision: "7",
    maxImportBytes: "134217728",
    skippedFiles: 0,
    records: [
      {
        id: "record-1",
        displayTime: "2026-07-31 20:00:00",
        recordedAt: "2026-07-31T12:00:00Z",
        hasDetails: true,
        partyLabel: "Character 1",
        canSetUpperPrediction: true,
        canSetLowerPrediction: true,
        summary: {
          durationSeconds: 10,
          dpsTimeBasis: "subtract_time_stop",
          totalDamage: 100,
          totalDps: 10,
          totalDamageTaken: 0,
          totalHits: "1",
          reactionDamageSeparated: false,
          characters: [],
          skills: [],
          hiddenCharacterCount: 0,
          hiddenSkillCount: 0,
          abyss: {
            detected: false,
            floor: null,
            activeHalf: null,
            success: false,
            firstHalf: null,
            secondHalf: null,
          },
          quality: {
            source: "live",
            packetCount: "3",
            hitCount: "1",
            unmappedSkillHits: "0",
            unknownCharacterHits: "0",
          },
        },
      },
    ],
  };
}

describe("History contract", () => {
  it("parses the bounded stable DTO", () => {
    const parsed = parseHistorySnapshot(snapshot());
    expect(parsed.records[0]?.summary.totalHits).toBe("1");
    expect(parsed.records[0]?.summary.quality.source).toBe("live");
  });

  it("rejects unknown versions and unsafe numeric counters", () => {
    expect(() =>
      parseHistorySnapshot({ ...snapshot(), contractVersion: 3 }),
    ).toThrow("Unsupported history contract version");
    const invalid = snapshot();
    invalid.records[0]!.summary.totalHits = "1e3";
    expect(() => parseHistorySnapshot(invalid)).toThrow("decimal string");
  });

  it("parses native file actions without exposing local paths", () => {
    const imported = parseHistoryImportFileResult({
      performed: true,
      importedRecordId: "record-1",
      history: snapshot(),
    });
    expect(imported.importedRecordId).toBe("record-1");
    expect(imported).not.toHaveProperty("path");
    expect(parseHistoryFileActionResult({ performed: false })).toEqual({
      performed: false,
    });
  });

  it("rejects an inline export beyond the WebView response budget", () => {
    expect(() =>
      parseHistoryExport({
        fileName: "history.json",
        json: "x".repeat(HISTORY_MAX_INLINE_EXPORT_CHARACTERS + 1),
      }),
    ).toThrow("history export.json");
  });
});
