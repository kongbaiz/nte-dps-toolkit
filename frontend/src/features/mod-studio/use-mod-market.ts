import { useCallback, useEffect, useRef, useState } from "react";

import {
  modStudioClient,
  type ModStudioClient,
} from "@/lib/tauri/mod-studio-client";
import {
  parseModStudioCommandError,
  type ModMarketCatalogSnapshot,
} from "@/lib/tauri/mod-studio-contract";

export type ModMarketState =
  | { status: "loading" }
  | { status: "ready"; catalog: ModMarketCatalogSnapshot }
  | {
      status: "error";
      error: ReturnType<typeof parseModStudioCommandError>;
    };

export type ModMarketInstallState =
  | { status: "idle" }
  | { status: "installing" }
  | { status: "enabling" }
  | { status: "installed" }
  | {
      status: "error";
      error: ReturnType<typeof parseModStudioCommandError>;
    };

export function useModMarket(
  onInstalled: () => void | Promise<void>,
  client: ModStudioClient = modStudioClient,
) {
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
      const catalog = await client.getMarketCatalog();
      if (activeRequest === request.current) {
        setState({ status: "ready", catalog });
      }
    } catch (error) {
      if (activeRequest === request.current) {
        setState({
          status: "error",
          error: parseModStudioCommandError(error),
        });
      }
    }
  }, [client]);

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
        await client.installMarketItem(id);
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
            error: parseModStudioCommandError(error),
          },
        }));
      } finally {
        installing.current.delete(id);
      }
    },
    [client, onInstalled, refresh],
  );

  const enable = useCallback(
    async (id: string) => {
      if (installing.current.has(id)) {
        return;
      }
      installing.current.add(id);
      setInstallStates((current) => ({
        ...current,
        [id]: { status: "enabling" },
      }));
      try {
        await client.setEnabled(id, true);
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
            error: parseModStudioCommandError(error),
          },
        }));
      } finally {
        installing.current.delete(id);
      }
    },
    [client, onInstalled, refresh],
  );

  useEffect(() => {
    void refresh();
    return () => {
      request.current += 1;
    };
  }, [refresh]);

  return { state, installStates, refresh, install, enable };
}
