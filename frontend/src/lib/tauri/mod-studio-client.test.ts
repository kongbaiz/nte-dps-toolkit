import { describe, expect, it, vi } from "vitest";

import { createModStudioClient } from "./mod-studio-client";

const workspace = {
  contractVersion: 4,
  workspaceLabel: "plugins/nte-mods",
  documents: [],
};

describe("Mod Studio client", () => {
  it("uses typed commands for workspace and on-demand source", async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(workspace)
      .mockResolvedValueOnce({
        contractVersion: 4,
        schemaVersion: 1,
        symbols: [
          {
            label: "nte::time::now_ms()",
            insertText: "nte::time::now_ms()",
            kind: "function",
            returnType: "std::uint64_t",
            documentationKey: "Monotonic process time in milliseconds.",
          },
        ],
      })
      .mockResolvedValueOnce({
        contractVersion: 4,
        id: "telemetry",
        enabled: false,
        source: "NTE_SCRIPT(5);",
      })
      .mockResolvedValueOnce({
        contractVersion: 4,
        id: "telemetry",
        enabled: false,
        source: "NTE_SCRIPT(5);\n// saved",
      })
      .mockResolvedValueOnce(workspace);
    const client = createModStudioClient({
      invoke,
      createChannel: vi.fn(),
    });

    await expect(client.getWorkspace()).resolves.toEqual(workspace);
    await expect(client.getSdkSchema()).resolves.toMatchObject({
      schemaVersion: 1,
      symbols: [{ label: "nte::time::now_ms()" }],
    });
    await expect(client.getDocument("telemetry")).resolves.toMatchObject({
      id: "telemetry",
      source: "NTE_SCRIPT(5);",
    });
    await expect(
      client.saveDocument("telemetry", "NTE_SCRIPT(5);\n// saved"),
    ).resolves.toMatchObject({
      id: "telemetry",
      source: "NTE_SCRIPT(5);\n// saved",
    });
    await expect(client.setEnabled("telemetry", true)).resolves.toEqual(
      workspace,
    );
    expect(invoke).toHaveBeenNthCalledWith(
      1,
      "get_mod_studio_workspace",
      undefined,
    );
    expect(invoke).toHaveBeenNthCalledWith(
      2,
      "get_mod_studio_sdk_schema",
      undefined,
    );
    expect(invoke).toHaveBeenNthCalledWith(3, "get_mod_studio_document", {
      id: "telemetry",
    });
    expect(invoke).toHaveBeenNthCalledWith(4, "save_mod_studio_document", {
      id: "telemetry",
      source: "NTE_SCRIPT(5);\n// saved",
    });
    expect(invoke).toHaveBeenNthCalledWith(
      5,
      "set_mod_studio_document_enabled",
      {
        id: "telemetry",
        enabled: true,
      },
    );
  });

  it("subscribes through a typed Channel and cleans it up", async () => {
    let deliver: ((message: unknown) => void) | undefined;
    const invoke = vi
      .fn()
      .mockResolvedValueOnce({
        subscriptionId: "runtime-01",
        streamIntervalMs: 250,
      })
      .mockResolvedValueOnce(undefined);
    const onEvent = vi.fn();
    const onError = vi.fn();
    const client = createModStudioClient(
      {
        invoke,
        createChannel: (onMessage) => {
          deliver = onMessage;
          return "runtime-channel";
        },
      },
      () => "runtime-01",
    );

    const unsubscribe = client.subscribeRuntime(onEvent, onError);
    deliver?.({
      event: "connection",
      payload: {
        contractVersion: 4,
        generation: "1",
        connected: true,
      },
    });
    await unsubscribe();

    expect(onEvent).toHaveBeenCalledWith({
      event: "connection",
      payload: {
        contractVersion: 4,
        generation: "1",
        connected: true,
      },
    });
    expect(onError).not.toHaveBeenCalled();
    expect(invoke).toHaveBeenNthCalledWith(1, "subscribe_mod_studio_runtime", {
      subscriptionId: "runtime-01",
      onEvent: "runtime-channel",
    });
    expect(invoke).toHaveBeenNthCalledWith(
      2,
      "unsubscribe_mod_studio_runtime",
      {
        subscriptionId: "runtime-01",
      },
    );
  });

  it("normalizes command failures at the client boundary", async () => {
    const client = createModStudioClient({
      invoke: vi.fn().mockRejectedValue({
        code: "mod_workspace_read_failed",
        messageKey: "Failed to read the Mod workspace.",
        messageArguments: [],
        diagnosticLine: 18,
      }),
      createChannel: vi.fn(),
    });

    await expect(client.getWorkspace()).rejects.toEqual({
      code: "mod_workspace_read_failed",
      messageKey: "Failed to read the Mod workspace.",
      messageArguments: [],
      diagnosticLine: 18,
    });
  });
});
