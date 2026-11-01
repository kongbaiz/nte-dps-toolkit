import { useCallback, useEffect, useRef, useState } from "react";

import { modStudioClient } from "@/lib/tauri/mod-studio-client";
import type { ModMarketCatalogSnapshot } from "@/lib/tauri/generated/ModMarketCatalogSnapshot";
import { toCommandError } from "@/lib/tauri/command-error";

export type ModMarketState =
  | { status: "loading" }
  | { status: "ready"; catalog: ModMarketCatalogSnapshot }
  | {
      status: "error";
      error: ReturnType<typeof toCommandError>;
    };

export type ModMarketInstallState =
  | { status: "idle" }
  | { status: "installing" }
  | { status: "installed" }
  | {
      status: "error";
      error: ReturnType<typeof toCommandError>;
    };

export function useModMarket(onInstalled: () => void | Promise<void>) {
  const request = useRef(0);
  const installing = useRef(new Set<string>());
  const [state, setState] = useState<ModMarketState>({ status: "loading" });
  const [installStates, setInstallStates] = useState<
    Record<string, ModMarketInstallState>
  >({});

  const refresh = useCallback(async () => {
    const activeRequest = ++request.current;
    setState({ status: "loading" });
    try {
      const catalog = await modStudioClient.getMarketCatalog();
      if (activeRequest === request.current) {
        setState({ status: "ready", catalog });
      }
    } catch (error) {
      if (activeRequest === request.current) {
        setState({
          status: "error",
          error: toCommandError(error),
        });
      }
    }
  }, []);

  const install = useCallback(
    async (id: string) => {
      if (installing.current.has(id)) {
        return;
      }
      installing.current.add(id);
      setInstallStates((current) => ({
        ...current,
        [id]: { status: "installing" },
      }));
      try {
        await modStudioClient.installMarketItem(id);
        setInstallStates((current) => ({
          ...current,
          [id]: { status: "installed" },
        }));
        await Promise.all([onInstalled(), refresh()]);
      } catch (error) {
        setInstallStates((current) => ({
          ...current,
          [id]: {
            status: "error",
            error: toCommandError(error),
          },
        }));
      } finally {
        installing.current.delete(id);
      }
    },
    [onInstalled, refresh],
  );

  useEffect(() => {
    void refresh();
    return () => {
      request.current += 1;
    };
  }, [refresh]);

  return { state, installStates, refresh, install };
}
