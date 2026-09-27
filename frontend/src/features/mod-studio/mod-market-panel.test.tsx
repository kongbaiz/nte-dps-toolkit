import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { marketFixture } from "../../../dev/fixtures";
import { setFrontendLanguage } from "@/lib/i18n";
import { parseModMarketCatalog } from "@/lib/tauri/mod-studio-contract";
import { ModMarketPanel } from "./mod-market-panel";

vi.mock("./use-mod-market", () => ({
  useModMarket: () => ({
    state: { status: "ready", catalog: marketFixture },
    installStates: {},
    refresh: vi.fn(),
    install: vi.fn(),
  }),
}));

describe("separate market categories", () => {
  it("uses a valid mixed catalog but defaults to feature plugins only", () => {
    expect(parseModMarketCatalog(marketFixture).mods).toHaveLength(7);
    setFrontendLanguage("en");
    const html = renderToStaticMarkup(
      <ModMarketPanel onInstalled={() => {}} />,
    );
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
});
