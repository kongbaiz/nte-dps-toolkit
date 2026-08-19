import { describe, expect, it } from "vitest";

import semverConformance from "@res/contract-semver-conformance.json";

import {
  ModStudioContractError,
  compareModStudioSequence,
  parseModMarketCatalog,
  parseModLoaderRuntime,
  parseModStudioCommandError,
  parseModStudioDeployment,
  parseModStudioDirectorySelection,
  parseModStudioDocument,
  parseModStudioGameDirectory,
  parseModStudioLoadingMethodPreference,
  parseModStudioRuntimeEvent,
  parseModStudioSdkSchema,
  parseModStudioSubscriptionReceipt,
  parseModStudioWorkspace,
} from "./mod-studio-contract";

function marketCatalogFixture() {
  return {
    contractVersion: 11,
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
        localState: { status: "notInstalled" },
      },
    ],
  };
}

describe("Mod Studio contract", () => {
  it("matches the shared canonical SemVer conformance vectors", () => {
    for (const vector of semverConformance) {
      const catalog = marketCatalogFixture();
      catalog.mods[0].version = vector.value;
      if (vector.valid) {
        expect(parseModMarketCatalog(catalog).mods[0].version).toBe(
          vector.value,
        );
      } else {
        expect(() => parseModMarketCatalog(catalog)).toThrow(
          ModStudioContractError,
        );
      }
    }
  });
  it("parses the persisted loading method preference", () => {
    expect(
      parseModStudioLoadingMethodPreference({
        contractVersion: 1,
        method: "loader",
        riskAcknowledged: true,
      }),
    ).toEqual({
      contractVersion: 1,
      method: "loader",
      riskAcknowledged: true,
    });
    expect(() =>
      parseModStudioLoadingMethodPreference({
        contractVersion: 1,
        method: "future",
        riskAcknowledged: false,
      }),
    ).toThrow(ModStudioContractError);
  });

  it("parses a bounded workspace index without source bodies", () => {
    expect(
      parseModStudioWorkspace({
        contractVersion: 10,
        generation: "7",
        workspaceLabel: "plugins/nte-mods",
        documents: [
          {
            id: "combat-clock",
            enabled: true,
            sourceBytes: 1200,
            lineCount: 42,
          },
        ],
      }),
    ).toEqual({
      contractVersion: 10,
      generation: "7",
      workspaceLabel: "plugins/nte-mods",
      documents: [
        {
          id: "combat-clock",
          enabled: true,
          sourceBytes: 1200,
          lineCount: 42,
        },
      ],
    });
  });

  it("rejects unknown versions, invalid IDs, and duplicate documents", () => {
    expect(() =>
      parseModStudioWorkspace({
        contractVersion: 8,
        generation: "0",
        workspaceLabel: "plugins/nte-mods",
        documents: [],
      }),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModStudioWorkspace({
        contractVersion: 10,
        generation: "0",
        workspaceLabel: "plugins/nte-mods",
        documents: [
          {
            id: "../combat-clock",
            enabled: true,
            sourceBytes: 1,
            lineCount: 1,
          },
        ],
      }),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModStudioWorkspace({
        contractVersion: 10,
        generation: "0",
        workspaceLabel: "plugins/nte-mods",
        documents: [
          {
            id: "combat-clock",
            enabled: true,
            sourceBytes: 1,
            lineCount: 1,
          },
          {
            id: "combat-clock",
            enabled: false,
            sourceBytes: 2,
            lineCount: 2,
          },
        ],
      }),
    ).toThrow(ModStudioContractError);
  });

  it("parses a requested document body", () => {
    expect(
      parseModStudioDocument({
        contractVersion: 10,
        id: "combat-clock",
        enabled: true,
        source: "NTE_SCRIPT(5);",
      }),
    ).toEqual({
      contractVersion: 10,
      id: "combat-clock",
      enabled: true,
      source: "NTE_SCRIPT(5);",
    });
  });

  it("parses privacy-bounded Mod Market catalog items", () => {
    expect(
      parseModMarketCatalog({
        contractVersion: 11,
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
            localState: { status: "notInstalled" },
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
            localState: {
              status: "installed",
              enabled: true,
              current: true,
            },
          },
        ],
      }),
    ).toMatchObject({
      privacyMode: "anonymous-read-only",
      mods: [
        {
          id: "combat-clock",
          bindings: ["feature.dps-time-stop"],
          packageSize: 1024,
          localizations: { "zh-CN": { summary: "追踪游戏时停。" } },
        },
        {
          id: "equipment",
          bindings: ["feature.empty-curtain-equipment"],
          localState: {
            status: "installed",
            enabled: true,
            current: true,
          },
        },
      ],
    });
    expect(() =>
      parseModMarketCatalog({
        contractVersion: 11,
        publishedAt: "2026-08-03T00:00:00Z",
        privacyMode: "tracks-device",
        mods: [],
      }),
    ).toThrow(ModStudioContractError);
  });

  it("preserves unreadable local Mod state without accepting private detail", () => {
    const parsed = parseModMarketCatalog({
      contractVersion: 11,
      publishedAt: "2026-08-03T00:00:00Z",
      privacyMode: "anonymous-read-only",
      mods: [
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
          localState: {
            status: "unreadable",
            code: "mod_workspace_invalid",
            messageKey: "The Mod workspace data is invalid.",
          },
        },
      ],
    });

    expect(parsed.mods[0]?.localState).toEqual({
      status: "unreadable",
      code: "mod_workspace_invalid",
      messageKey: "The Mod workspace data is invalid.",
    });
    expect(JSON.stringify(parsed)).not.toContain("detail");
  });

  it("rejects incomplete or legacy Mod Market local state", () => {
    const item = {
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
    };
    const catalog = (localState: unknown) => ({
      contractVersion: 11,
      publishedAt: "2026-08-03T00:00:00Z",
      privacyMode: "anonymous-read-only",
      mods: [{ ...item, localState }],
    });

    expect(() =>
      parseModMarketCatalog(catalog({ status: "installed", enabled: true })),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModMarketCatalog(
        catalog({ status: "unreadable", code: "mod_workspace_invalid" }),
      ),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModMarketCatalog(
        catalog({ status: "notInstalled", enabled: false }),
      ),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModMarketCatalog(
        catalog({
          status: "installed",
          enabled: false,
          current: true,
          code: "mod_workspace_invalid",
        }),
      ),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModMarketCatalog(
        catalog({
          status: "unreadable",
          code: "mod_workspace_invalid",
          messageKey: "The Mod workspace data is invalid.",
          current: false,
        }),
      ),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModMarketCatalog(
        catalog({
          status: "unreadable",
          code: "mod_workspace_future_error",
          messageKey: "The Mod workspace data is invalid.",
        }),
      ),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModMarketCatalog(
        catalog({
          status: "unreadable",
          code: "mod_workspace_invalid",
          messageKey: "Failed to read the Mod workspace.",
        }),
      ),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModMarketCatalog({
        ...catalog(undefined),
        mods: [{ ...item, installed: true, enabled: false, current: false }],
      }),
    ).toThrow(ModStudioContractError);
  });

  it("parses bounded deployment state and a validated manual path selection", () => {
    const deployment = {
      contractVersion: 10,
      installations: 1,
      installed: 1,
      current: 1,
      sourceAvailable: true,
      games: [{ region: "global", installed: true, current: true }],
    };

    expect(parseModStudioDeployment(deployment)).toEqual(deployment);
    expect(
      parseModStudioDirectorySelection({
        selected: true,
        path: "D:\\Game",
        deployment,
      }),
    ).toMatchObject({ selected: true, path: "D:\\Game" });
    expect(() =>
      parseModStudioDirectorySelection({
        selected: false,
        path: "D:\\Game",
        deployment,
      }),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModStudioDeployment({
        ...deployment,
        installed: 2,
      }),
    ).toThrow(ModStudioContractError);
  });

  it("parses a persisted per-region game directory preference", () => {
    expect(
      parseModStudioGameDirectory({
        contractVersion: 1,
        region: "china",
        path: "D:\\CustomGame",
      }),
    ).toEqual({
      contractVersion: 1,
      region: "china",
      path: "D:\\CustomGame",
    });
    expect(
      parseModStudioGameDirectory({
        contractVersion: 1,
        region: "global",
        path: null,
      }).path,
    ).toBeNull();
    expect(() =>
      parseModStudioGameDirectory({
        contractVersion: 2,
        region: "china",
        path: null,
      }),
    ).toThrow(ModStudioContractError);
  });

  it("enforces the managed Mod Loader placement and runtime invariants", () => {
    const running = {
      contractVersion: 1,
      phase: "running",
      loaderPresent: true,
      payloadPresent: true,
      loaderFileName: "nte-mod-loader.exe",
      payloadRelativePath: "plugins/dwmapi.dll",
      placement: "applicationDirectory",
    };
    expect(parseModLoaderRuntime(running)).toEqual(running);
    expect(() =>
      parseModLoaderRuntime({
        ...running,
        loaderFileName: "dwmapi.dll",
      }),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModLoaderRuntime({
        ...running,
        phase: "missingPayload",
      }),
    ).toThrow(ModStudioContractError);
  });

  it("parses the bounded versioned Mod SDK schema", () => {
    expect(
      parseModStudioSdkSchema({
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
      }),
    ).toMatchObject({
      schemaVersion: 2,
      symbols: [{ kind: "function", returnType: "std::uint64_t" }],
    });
    expect(() =>
      parseModStudioSdkSchema({
        contractVersion: 10,
        schemaVersion: 1,
        symbols: [],
      }),
    ).toThrow(ModStudioContractError);
  });

  it("parses optional source diagnostics and rejects invalid diagnostic lines", () => {
    expect(
      parseModStudioCommandError({
        code: "mod_source_invalid_line",
        messageKey: "The NTE C++ compiler rejected line {0}.",
        messageArguments: ["23"],
        diagnosticLine: 23,
      }),
    ).toEqual({
      code: "mod_source_invalid_line",
      messageKey: "The NTE C++ compiler rejected line {0}.",
      messageArguments: ["23"],
      diagnosticLine: 23,
    });
    expect(() =>
      parseModStudioCommandError({
        code: "mod_source_invalid_line",
        messageKey: "The NTE C++ compiler rejected line {0}.",
        messageArguments: ["0"],
        diagnosticLine: 0,
      }),
    ).toThrow(ModStudioContractError);
    expect(
      parseModStudioCommandError({
        code: "mod_source_write_failed",
        messageKey: "Failed to save the Mod source.",
        messageArguments: [],
        diagnosticLine: null,
      }).diagnosticLine,
    ).toBeNull();
  });

  it("parses ordered bounded runtime batches with u64 string fields", () => {
    expect(
      parseModStudioRuntimeEvent({
        event: "batch",
        payload: {
          contractVersion: 10,
          generation: "2",
          entries: [
            {
              kind: "log",
              sequence: "9",
              nativeSequence: "4",
              timestamp100ns: "133000000000000000",
              modId: "runtime",
              level: "warning",
              message: "Compilation failed; previous version kept.",
              messageKey: "Compilation failed; previous version kept.",
              messageArguments: [],
            },
            {
              kind: "event",
              sequence: "10",
              nativeSequence: "7",
              timestamp100ns: "133000000000000010",
              modId: "telemetry",
              name: "post.sample",
              values: ["1", "18446744073709551615"],
            },
          ],
        },
      }),
    ).toMatchObject({
      event: "batch",
      payload: {
        generation: "2",
        entries: [{ sequence: "9" }, { sequence: "10" }],
      },
    });
    expect(() =>
      parseModStudioRuntimeEvent({
        event: "batch",
        payload: {
          contractVersion: 10,
          generation: "2",
          entries: [
            {
              kind: "log",
              sequence: "10",
              nativeSequence: "10",
              timestamp100ns: "1",
              modId: "runtime",
              level: "info",
              message: "Hot reload applied.",
              messageKey: "Hot reload applied.",
              messageArguments: [],
            },
            {
              kind: "log",
              sequence: "9",
              nativeSequence: "11",
              timestamp100ns: "2",
              modId: "runtime",
              level: "info",
              message: "Hot reload applied.",
              messageKey: "Hot reload applied.",
              messageArguments: [],
            },
          ],
        },
      }),
    ).toThrow(ModStudioContractError);
    expect(compareModStudioSequence("9", "10")).toBeLessThan(0);
  });

  it("parses runtime connection receipts", () => {
    expect(
      parseModStudioSubscriptionReceipt({
        subscriptionId: "runtime-01",
        streamKind: "modStudioRuntime",
        streamIntervalMs: 250,
        streamProtocolVersion: 1,
        streamGeneration: "3",
        maxInFlightDeliveries: 1,
        maxDeliveryBytes: 16_777_216,
      }),
    ).toEqual({
      subscriptionId: "runtime-01",
      streamKind: "modStudioRuntime",
      streamIntervalMs: 250,
      streamProtocolVersion: 1,
      streamGeneration: "3",
      maxInFlightDeliveries: 1,
      maxDeliveryBytes: 16_777_216,
    });
    expect(
      parseModStudioRuntimeEvent({
        event: "connection",
        payload: {
          contractVersion: 10,
          generation: "3",
          status: "waiting",
        },
      }),
    ).toMatchObject({
      event: "connection",
      payload: { generation: "3", status: "waiting" },
    });
    expect(() =>
      parseModStudioRuntimeEvent({
        event: "connection",
        payload: {
          contractVersion: 10,
          generation: "3",
          status: "installed",
        },
      }),
    ).toThrow(ModStudioContractError);
  });
});
