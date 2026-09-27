import {
  parseModMarketCatalog,
  ModStudioContractError,
  type ModMarketCatalogSnapshot,
} from "./mod-studio-contract";
import { tauriInvokeTransport, type InvokeTransport } from "./stream-client";
export interface ModStudioClient {
  getMarketCatalog(): Promise<ModMarketCatalogSnapshot>;
  installMarketItem(id: string): Promise<true>;
}
export function createModStudioClient(
  transport: InvokeTransport = tauriInvokeTransport,
): ModStudioClient {
  return {
    async getMarketCatalog() {
      return parseModMarketCatalog(
        await transport.invoke("get_mod_market_catalog"),
      );
    },
    async installMarketItem(id: string) {
      if ((await transport.invoke("install_mod_market_item", { id })) !== true)
        throw new ModStudioContractError("invalid plugin installation receipt");
      return true;
    },
  };
}
export const modStudioClient = createModStudioClient();
