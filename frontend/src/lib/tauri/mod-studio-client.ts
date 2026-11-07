import { invokeCommand } from "@/lib/tauri/command-error";
import type { ModMarketCatalogSnapshot } from "@/lib/tauri/generated/ModMarketCatalogSnapshot";

export const modStudioClient = {
  getMarketCatalog: () =>
    invokeCommand<ModMarketCatalogSnapshot>("get_mod_market_catalog"),
  installMarketItem: (id: string) =>
    invokeCommand<true>("install_mod_market_item", { id }),
};
