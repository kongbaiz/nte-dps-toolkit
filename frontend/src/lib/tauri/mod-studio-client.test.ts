import { describe, expect, it, vi } from "vitest";

import { createModStudioClient } from "./mod-studio-client";

const workspace = {
  contractVersion: 10,
  generation: "0",
  workspaceLabel: "plugins/nte-mods",
  documents: [],
};

describe("Mod Studio client", () => {
  it("loads and clears the persisted game directory preference", async () => {
    const preference = {
      contractVersion: 1,
      region: "china" as const,
      path: "D:\\CustomGame",
    };
    const cleared = { ...preference, path: null };
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(preference)
      .mockResolvedValueOnce(cleared);
    const client = createModStudioClient({ invoke, createChannel: vi.fn() });

    await expect(client.getGameDirectory("china")).resolves.toEqual(preference);
    await expect(client.setGameDirectory("china", null)).resolves.toEqual(
      cleared,
    );
    expect(invoke).toHaveBeenNthCalledWith(1, "get_mod_studio_game_directory", {
      region: "china",
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "set_mod_studio_game_directory", {
      region: "china",
      gameDirectory: null,
    });
  });

  it("deletes a workspace Mod through the typed Rust command", async () => {
    const deletedWorkspace = { ...workspace, generation: "4" };
    const invoke = vi.fn().mockResolvedValueOnce(deletedWorkspace);
    const client = createModStudioClient({ invoke, createChannel: vi.fn() });

    await expect(client.deleteDocument("telemetry")).resolves.toEqual(
      deletedWorkspace,
    );
    expect(invoke).toHaveBeenCalledWith("delete_mod_studio_document", {
      id: "telemetry",
    });
  });

  it("loads and installs market items through Rust-owned commands", async () => {
    const catalog = {
      contractVersion: 10,
      publishedAt: "2026-08-03T00:00:00Z",
      privacyMode: "anonymous-read-only",
      mods: [
        {
          id: "combat-clock",
          bindings: ["feature.dps-time-stop"],
          localizations: {
            en: { name: "Combat Clock", summary: "Tracks game pauses." },
            "zh-CN": { name: "时停扣除", summary: "追踪游戏时停。" },
            ja: { name: "時間停止控除", summary: "停止を追跡します。" },
          },
          version: "1.0.0",
          author: "NTE",
          capabilities: ["combat-clock", "ipc"],
          packageSize: 1024,
          installed: false,
          enabled: false,
          current: false,
        },
        {
          id: "equipment",
          bindings: ["feature.empty-curtain-equipment"],
          localizations: {
            en: { name: "Equipment", summary: "Manages equipment." },
            "zh-CN": { name: "空幕装备", summary: "管理空幕装备。" },
            ja: { name: "空幕装備", summary: "装備を管理します。" },
          },
          version: "1.0.0",
          author: "NTE",
          capabilities: ["equipment", "ipc"],
          packageSize: 2048,
          installed: true,
          enabled: true,
          current: true,
        },
      ],
    };
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(catalog)
      .mockResolvedValueOnce({
        contractVersion: 10,
        id: "combat-clock",
        enabled: true,
        source: "NTE_SCRIPT(5);",
      });
    const client = createModStudioClient({ invoke, createChannel: vi.fn() });

    await expect(client.getMarketCatalog()).resolves.toEqual(catalog);
    await expect(
      client.installMarketItem("combat-clock"),
    ).resolves.toMatchObject({ id: "combat-clock", enabled: true });
    expect(invoke).toHaveBeenNthCalledWith(
      1,
      "get_mod_market_catalog",
      undefined,
    );
    expect(invoke).toHaveBeenNthCalledWith(2, "install_mod_market_item", {
      id: "combat-clock",
    });
  });

  it("routes creation, folders, manual game selection, and loader deployment through typed commands", async () => {
    const deployment = {
      contractVersion: 10,
      installations: 1,
      installed: 0,
      current: 0,
      sourceAvailable: true,
      games: [{ region: "global", installed: false, current: false }],
    };
    const invoke = vi
      .fn()
      .mockResolvedValueOnce({
        contractVersion: 10,
        id: "telemetry",
        enabled: false,
        source: "NTE_SCRIPT(5);",
      })
      .mockResolvedValueOnce(true)
      .mockResolvedValueOnce(deployment)
      .mockResolvedValueOnce({
        selected: true,
        path: "D:\\Game",
        deployment,
      })
      .mockResolvedValueOnce({
        ...deployment,
        installed: 1,
        current: 1,
        games: [{ region: "global", installed: true, current: true }],
      });
    const client = createModStudioClient({ invoke, createChannel: vi.fn() });

    await client.createDocument("telemetry");
    await client.openFolder();
    await client.getDeployment("global", "D:\\Game");
    await client.chooseGameDirectory("global");
    await client.setLoaderEnabled("global", true, "D:\\Game");

    expect(invoke).toHaveBeenNthCalledWith(1, "create_mod_studio_document", {
      id: "telemetry",
    });
    expect(invoke).toHaveBeenNthCalledWith(
      2,
      "open_mod_studio_folder",
      undefined,
    );
    expect(invoke).toHaveBeenNthCalledWith(3, "get_mod_studio_deployment", {
      region: "global",
      gameDirectory: "D:\\Game",
    });
    expect(invoke).toHaveBeenNthCalledWith(
      4,
      "choose_mod_studio_game_directory",
      { region: "global" },
    );
    expect(invoke).toHaveBeenNthCalledWith(5, "set_mod_studio_loader_enabled", {
      region: "global",
      enabled: true,
      gameDirectory: "D:\\Game",
    });
  });

  it("uses typed commands for workspace and on-demand source", async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(workspace)
      .mockResolvedValueOnce({
        contractVersion: 10,
        schemaVersion: 2,
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
        contractVersion: 10,
        id: "telemetry",
        enabled: false,
        source: "NTE_SCRIPT(5);",
      })
      .mockResolvedValueOnce({
        contractVersion: 10,
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
      schemaVersion: 2,
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
        contractVersion: 10,
        generation: "1",
        status: "connected",
      },
    });
    await unsubscribe();

    expect(onEvent).toHaveBeenCalledWith({
      event: "connection",
      payload: {
        contractVersion: 10,
        generation: "1",
        status: "connected",
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
