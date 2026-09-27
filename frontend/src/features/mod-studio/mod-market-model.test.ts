import { describe, expect, it } from "vitest";

import type { ModMarketItem } from "@/lib/tauri/mod-studio-contract";

import {
  localizedModMarketText,
  filterModMarketItems,
  modMarketLocalStatus,
  modMarketSearchText,
} from "./mod-market-model";

const ITEM: ModMarketItem = {
  id: "combat-clock",
  bindings: ["feature.dps-time-stop"],
  localizations: {
    en: { name: "Sample", summary: "English description" },
    "zh-CN": { name: "示例", summary: "中文功能描述" },
    ja: { name: "サンプル", summary: "日本語の機能説明" },
  },
  version: "1.0.0",
  author: "NTE",
  capabilities: ["viewport.tick"],
  component: "plugin",
  packageSize: 1024,
  localState: { status: "notInstalled" },
};

describe("Mod Market localization", () => {
  it("selects the description matching the active application language", () => {
    expect(localizedModMarketText(ITEM, "en")).toEqual({
      name: "Sample",
      summary: "English description",
    });
    expect(localizedModMarketText(ITEM, "zh-CN").summary).toBe("中文功能描述");
    expect(localizedModMarketText(ITEM, "ja").summary).toBe("日本語の機能説明");
  });

  it("searches only the current locale plus stable identity fields", () => {
    expect(modMarketSearchText(ITEM, "zh-CN")).toContain("中文功能描述");
    expect(modMarketSearchText(ITEM, "zh-CN")).toContain(
      "feature.dps-time-stop",
    );
    expect(modMarketSearchText(ITEM, "zh-CN")).not.toContain(
      "English description",
    );
  });

  it("keeps unreadable state distinct from installed or disabled", () => {
    expect(
      modMarketLocalStatus({
        ...ITEM,
        localState: {
          status: "unreadable",
          code: "mod_workspace_read_failed",
          messageKey: "Failed to read the Mod workspace.",
        },
      }),
    ).toEqual({
      installed: false,
      enabled: false,
      current: false,
      unreadable: {
        code: "mod_workspace_read_failed",
        messageKey: "Failed to read the Mod workspace.",
      },
    });
  });
});

describe("market component separation", () => {
  const items: ModMarketItem[] = (
    ["host", "plugin", "loader", "driver"] as const
  ).map((component) => ({ ...ITEM, id: component, component }));
  it("keeps the shared runtime out of feature plugin results", () => {
    expect(
      filterModMarketItems(items, "plugins", "", "en").map(
        (item) => item.component,
      ),
    ).toEqual(["plugin"]);
    expect(
      filterModMarketItems(items, "foundation", "", "en").map(
        (item) => item.component,
      ),
    ).toEqual(["host", "loader", "driver"]);
  });
  it("does not allow a search result to cross categories", () => {
    expect(filterModMarketItems(items, "plugins", "host", "en")).toEqual([]);
    expect(filterModMarketItems(items, "foundation", "plugin", "en")).toEqual(
      [],
    );
    expect(
      filterModMarketItems(items, "foundation", "  LOADER  ", "en").map(
        (item) => item.id,
      ),
    ).toEqual(["loader"]);
  });
  it("classifies by the contract component kind, never by names or IDs", () => {
    const plugin = { ...ITEM, id: "nte-host", component: "plugin" as const };
    expect(filterModMarketItems([plugin], "plugins", "", "en")).toEqual([
      plugin,
    ]);
    expect(filterModMarketItems([plugin], "foundation", "", "en")).toEqual([]);
  });
});
