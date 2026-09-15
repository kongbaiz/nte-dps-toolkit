import { describe, it, expect, vi } from "vitest";
import { createModStudioClient } from "./mod-studio-client";
describe("compiled plugin market client", () => {
  it("requires a binary installation receipt and never routes to source saving", async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(true)
      .mockResolvedValueOnce({ source: "old source" });
    const client = createModStudioClient({ invoke });
    await expect(client.installMarketItem("combat")).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith("install_mod_market_item", {
      id: "combat",
    });
    await expect(client.installMarketItem("combat")).rejects.toThrow();
    expect(Object.keys(client).sort()).toEqual([
      "getMarketCatalog",
      "installMarketItem",
    ]);
  });
});
