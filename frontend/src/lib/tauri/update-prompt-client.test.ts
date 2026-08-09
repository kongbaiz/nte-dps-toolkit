import { describe, expect, it } from "vitest";

import { createUpdatePromptClient } from "./update-prompt-client";

const promptFixture = () => ({
  contractVersion: 1,
  updates: {
    currentVersion: "0.3.7",
    autoCheck: true,
    autoDownload: false,
    status: "available",
    messageKey: "Version {} is available",
    messageArguments: ["0.3.8"],
    available: [],
    activeComponent: null,
    downloadedBytes: "0",
    totalBytes: "0",
    prepared: null,
    installEnabled: false,
    installBlockedMessageKey: null,
  },
});

describe("update prompt client", () => {
  it("routes startup prompt commands through the typed boundary", async () => {
    const calls: Array<{
      command: string;
      arguments_?: Record<string, unknown>;
    }> = [];
    const client = createUpdatePromptClient({
      invoke: async (command, arguments_) => {
        calls.push({ command, arguments_ });
        return promptFixture();
      },
      listen: async () => () => {},
    });

    await client.get();
    await client.download("app");
    await client.install();

    expect(calls).toEqual([
      { command: "get_main_dps_update_prompt", arguments_: undefined },
      {
        command: "download_main_dps_update",
        arguments_: { component: "app" },
      },
      { command: "install_main_dps_update", arguments_: undefined },
    ]);
  });
});
