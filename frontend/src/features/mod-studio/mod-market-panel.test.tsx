import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { marketFixture } from "../../../dev/fixtures";
import { setFrontendLanguage } from "@/lib/i18n";
import { parseModMarketCatalog } from "@/lib/tauri/mod-studio-contract";
import { ModMarketPanel } from "./mod-market-panel";

const market = {
  state: { status: "ready" as const, catalog: marketFixture },
  installStates: {},
  refresh: vi.fn(),
  install: vi.fn(),
};

describe("separate market categories", () => {
  it("uses a valid mixed catalog but defaults to feature plugins only", () => {
    expect(parseModMarketCatalog(marketFixture).mods).toHaveLength(7);
    setFrontendLanguage("en");
    const html = renderToStaticMarkup(<ModMarketPanel market={market} />);
    expect(html).toContain("Foundation components");
    expect(html).toContain("Search feature plugins");
    const cards = html.match(/<article\b[^>]*>[\s\S]*?<\/article>/g) ?? [];
    expect(cards).toHaveLength(4);
    for (const card of cards) {
      expect(card).toContain("Plugin component");
      expect(card).toContain("Download plugin");
      expect(card).not.toMatch(/nte-host|nte-loader|uetools-driver/);
    }
  });
  it("marks only installed components with different verified content", () => {
    const catalog = {
      ...marketFixture,
      mods: marketFixture.mods.map((item) => ({
        ...item,
        localState: {
          status: "installed" as const,
          current: item.component !== "plugin",
          enabled: true,
        },
      })),
    };
    const html = renderToStaticMarkup(
      <ModMarketPanel
        market={{ ...market, state: { status: "ready", catalog } }}
      />,
    );
    expect(html).toContain('aria-label="Updates available"');
    expect(html.match(/aria-label="Update available"/g)).toHaveLength(4);
    const updated = {
      ...catalog,
      mods: catalog.mods.map((item) => ({
        ...item,
        localState: { ...item.localState, current: true },
      })),
    };
    expect(
      renderToStaticMarkup(
        <ModMarketPanel
          market={{ ...market, state: { status: "ready", catalog: updated } }}
        />,
      ),
    ).not.toContain('aria-label="Update available"');
  });
});
