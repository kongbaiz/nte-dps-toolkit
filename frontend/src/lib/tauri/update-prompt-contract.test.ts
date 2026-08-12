import { describe, expect, it } from "vitest";

import {
  parseUpdatePromptSnapshot,
  UPDATE_PROMPT_CONTRACT_VERSION,
} from "./update-prompt-contract";

describe("update prompt contract", () => {
  it("parses the bounded update projection", () => {
    const snapshot = parseUpdatePromptSnapshot({
      contractVersion: UPDATE_PROMPT_CONTRACT_VERSION,
      updates: {
        currentVersion: "0.3.7",
        autoCheck: true,
        autoDownload: false,
        status: "available",
        messageKey: "Version {} is available",
        messageArguments: ["0.3.8"],
        available: [
          {
            component: "app",
            version: "0.3.8",
            publishedAt: "2026-08-09T00:00:00Z",
            notes: "## English\n\nEnglish update",
            artifactSize: "1024",
          },
        ],
        activeComponent: null,
        downloadedBytes: "0",
        totalBytes: "0",
        prepared: null,
        installEnabled: false,
        installBlockedMessageKey: null,
      },
    });

    expect(snapshot.updates.available[0]?.notes).toContain("English update");
  });

  it("rejects a future contract version", () => {
    expect(() =>
      parseUpdatePromptSnapshot({
        contractVersion: UPDATE_PROMPT_CONTRACT_VERSION + 1,
        updates: {},
      }),
    ).toThrow(/Unsupported update prompt contract/);
  });
});
