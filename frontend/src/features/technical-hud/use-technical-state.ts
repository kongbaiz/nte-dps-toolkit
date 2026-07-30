import { useCallback, useEffect, useState } from "react";

import {
  technicalClient,
  type TechnicalClient,
} from "@/lib/tauri/technical-client";
import {
  parseTechnicalCommandError,
  type TechnicalSnapshot,
} from "@/lib/tauri/technical-contract";

import {
  acceptSnapshot,
  type TechnicalPageState,
} from "./technical-view-model";

export function useTechnicalState(client: TechnicalClient = technicalClient) {
  const [state, setState] = useState<TechnicalPageState>({
    status: "loading",
  });

  const applySnapshot = useCallback((snapshot: TechnicalSnapshot) => {
    setState((current) => acceptSnapshot(current, snapshot));
  }, []);

  const applyError = useCallback((error: unknown) => {
    setState({
      status: "error",
      error: parseTechnicalCommandError(error),
    });
  }, []);

  const refresh = useCallback(async () => {
    try {
      applySnapshot(await client.getSnapshot());
    } catch (error) {
      applyError(error);
    }
  }, [applyError, applySnapshot, client]);

  const setPassthrough = useCallback(
    async (enabled: boolean) => {
      try {
        applySnapshot(await client.setPassthrough(enabled));
      } catch (error) {
        applyError(error);
      }
    },
    [applyError, applySnapshot, client],
  );

  const setAlwaysOnTop = useCallback(
    async (enabled: boolean) => {
      try {
        applySnapshot(await client.setAlwaysOnTop(enabled));
      } catch (error) {
        applyError(error);
      }
    },
    [applyError, applySnapshot, client],
  );

  useEffect(() => {
    void refresh();
    const unsubscribe = client.subscribe(applySnapshot, applyError);

    return () => {
      void unsubscribe().catch((error: unknown) => {
        console.error("technical subscription cleanup failed", error);
      });
    };
  }, [applyError, applySnapshot, client, refresh]);

  return {
    state,
    refresh,
    setPassthrough,
    setAlwaysOnTop,
  };
}
