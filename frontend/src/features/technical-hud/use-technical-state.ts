import { useCallback, useEffect, useRef, useState } from "react";

import {
  technicalClient,
  type TechnicalClient,
} from "@/lib/tauri/technical-client";
import {
  parseTechnicalCommandError,
  type HudModuleId,
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
  const [actionNotice, setActionNotice] = useState<{
    id: number;
    action: string;
    status: "pending" | "success";
  } | null>(null);
  const actionId = useRef(0);

  const applySnapshot = useCallback((snapshot: TechnicalSnapshot) => {
    setState((current) => acceptSnapshot(current, snapshot));
  }, []);

  const applyError = useCallback((error: unknown) => {
    setState({
      status: "error",
      error: parseTechnicalCommandError(error),
    });
  }, []);

  useEffect(() => {
    if (actionNotice?.status !== "success") return;
    const timer = window.setTimeout(() => setActionNotice(null), 1_800);
    return () => window.clearTimeout(timer);
  }, [actionNotice]);

  const runAction = useCallback(
    async (action: string, request: () => Promise<TechnicalSnapshot>) => {
      const id = ++actionId.current;
      setActionNotice({ id, action, status: "pending" });
      try {
        applySnapshot(await request());
        if (actionId.current === id) {
          setActionNotice({ id, action, status: "success" });
        }
      } catch (error) {
        if (actionId.current === id) setActionNotice(null);
        applyError(error);
      }
    },
    [applyError, applySnapshot],
  );

  const refresh = useCallback(async () => {
    await runAction("refresh", () => client.getSnapshot());
  }, [client, runAction]);

  const setPassthrough = useCallback(
    async (enabled: boolean) => {
      await runAction("passthrough", () => client.setPassthrough(enabled));
    },
    [client, runAction],
  );

  const setAlwaysOnTop = useCallback(
    async (enabled: boolean) => {
      await runAction("always-on-top", () => client.setAlwaysOnTop(enabled));
    },
    [client, runAction],
  );

  const setHudModuleVisibility = useCallback(
    async (module: HudModuleId, visible: boolean) => {
      await runAction("hud-modules", () =>
        client.setModuleVisibility(module, visible),
      );
    },
    [client, runAction],
  );

  const moveHudModule = useCallback(
    async (dragged: HudModuleId, target: HudModuleId, insertAfter: boolean) => {
      await runAction("module-order", () =>
        client.moveModule(dragged, target, insertAfter),
      );
    },
    [client, runAction],
  );

  const setHudWidth = useCallback(
    async (width: number) => {
      await runAction("hud-width", () => client.setWidth(width));
    },
    [client, runAction],
  );

  const startCapture = useCallback(async () => {
    await runAction("start-capture", () => client.startCapture());
  }, [client, runAction]);

  const stopCapture = useCallback(async () => {
    await runAction("stop-capture", () => client.stopCapture());
  }, [client, runAction]);

  const clearActionNotice = useCallback(() => setActionNotice(null), []);

  useEffect(() => {
    void client.getSnapshot().then(applySnapshot).catch(applyError);
    const unsubscribe = client.subscribe(applySnapshot, applyError);

    return () => {
      void unsubscribe().catch((error: unknown) => {
        console.error("technical subscription cleanup failed", error);
      });
    };
  }, [applyError, applySnapshot, client]);

  return {
    state,
    actionNotice,
    clearActionNotice,
    refresh,
    setPassthrough,
    setAlwaysOnTop,
    setHudModuleVisibility,
    moveHudModule,
    setHudWidth,
    startCapture,
    stopCapture,
  };
}
