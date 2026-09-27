import { describe, expect, it } from "vitest";
import semverConformance from "@res/contract-semver-conformance.json";
import {
  ModStudioContractError,
  parseModMarketCatalog,
  parseModStudioCommandError,
} from "./mod-studio-contract";

function marketCatalogFixture() {
  return {
    contractVersion: 13,
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
        component: "plugin",
        packageSize: 1024,
        localState: { status: "notInstalled" },
      },
    ],
  };
}

describe("Mod Market contract", () => {
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
  it("parses privacy-bounded Mod Market catalog items", () => {
    expect(
      parseModMarketCatalog({
        contractVersion: 13,
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
            component: "plugin",
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
            component: "plugin",
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
          component: "plugin",
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
        contractVersion: 13,
        publishedAt: "2026-08-03T00:00:00Z",
        privacyMode: "tracks-device",
        mods: [],
      }),
    ).toThrow(ModStudioContractError);
  });

  it("preserves unreadable local Mod state without accepting private detail", () => {
    const parsed = parseModMarketCatalog({
      contractVersion: 13,
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
          component: "plugin",
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
      component: "plugin",
      packageSize: 2048,
    };
    const catalog = (localState: unknown) => ({
      contractVersion: 13,
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

  it("rejects unsupported market versions, invalid IDs, duplicates and oversized catalogs", () => {
    const catalog = marketCatalogFixture();
    for (const contractVersion of [12, 14, undefined]) {
      expect(() =>
        parseModMarketCatalog({ ...catalog, contractVersion }),
      ).toThrow(ModStudioContractError);
    }
    expect(() =>
      parseModMarketCatalog({
        ...catalog,
        mods: [{ ...catalog.mods[0], id: "../host" }],
      }),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModMarketCatalog({
        ...catalog,
        mods: [catalog.mods[0], catalog.mods[0]],
      }),
    ).toThrow(ModStudioContractError);
    expect(() => parseModMarketCatalog({ ...catalog, mods: [] })).toThrow(
      ModStudioContractError,
    );
    expect(() =>
      parseModMarketCatalog({
        ...catalog,
        mods: Array.from({ length: 65 }, (_, i) => ({
          ...catalog.mods[0],
          id: `plugin-${i}`,
        })),
      }),
    ).toThrow(ModStudioContractError);
  });
});

it("requires explicit market component kinds", () => {
  const item = {
    id: "nte-host",
    component: "host",
    bindings: ["toolkit.host"],
    localizations: {
      en: { name: "Host", summary: "Host" },
      "zh-CN": { name: "Host", summary: "Host" },
      ja: { name: "Host", summary: "Host" },
    },
    version: "1.0.0",
    author: "NTE",
    capabilities: [],
    packageSize: 192,
    localState: { status: "notInstalled" },
  };
  const catalog = {
    contractVersion: 13,
    publishedAt: "2026-09-27",
    privacyMode: "anonymous-read-only",
    mods: [item],
  };
  for (const component of ["host", "loader", "driver", "plugin"]) {
    expect(
      parseModMarketCatalog({ ...catalog, mods: [{ ...item, component }] })
        .mods[0]?.component,
    ).toBe(component);
  }
  for (const component of [undefined, "script", "../host"]) {
    expect(() =>
      parseModMarketCatalog({ ...catalog, mods: [{ ...item, component }] }),
    ).toThrow();
  }
});
