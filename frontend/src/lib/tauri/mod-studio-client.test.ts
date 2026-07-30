import { describe, expect, it, vi } from "vitest";

import { createModStudioClient } from "./mod-studio-client";

const workspace = {
  contractVersion: 1,
  workspaceLabel: "plugins/nte-mods",
  documents: [],
};

describe("Mod Studio client", () => {
  it("uses typed commands for workspace and on-demand source", async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(workspace)
      .mockResolvedValueOnce({
        contractVersion: 1,
        id: "telemetry",
        enabled: false,
        source: "NTE_SCRIPT(5);",
      });
    const client = createModStudioClient({ invoke });

    await expect(client.getWorkspace()).resolves.toEqual(workspace);
    await expect(client.getDocument("telemetry")).resolves.toMatchObject({
      id: "telemetry",
      source: "NTE_SCRIPT(5);",
    });
    expect(invoke).toHaveBeenNthCalledWith(
      1,
      "get_mod_studio_workspace",
      undefined,
    );
    expect(invoke).toHaveBeenNthCalledWith(2, "get_mod_studio_document", {
      id: "telemetry",
    });
  });

  it("normalizes command failures at the client boundary", async () => {
    const client = createModStudioClient({
      invoke: vi.fn().mockRejectedValue({
        code: "mod_workspace_read_failed",
        messageKey: "Failed to read the Mod workspace.",
        messageArguments: [],
      }),
    });

    await expect(client.getWorkspace()).rejects.toEqual({
      code: "mod_workspace_read_failed",
      messageKey: "Failed to read the Mod workspace.",
      messageArguments: [],
    });
  });
});
