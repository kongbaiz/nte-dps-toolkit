import { useCallback, useEffect, useRef, useState } from "react";

import { technicalClient } from "@/lib/tauri/technical-client";
import type { HudModule } from "@/lib/tauri/generated/HudModule";
import type { TechnicalSnapshot } from "@/lib/tauri/generated/TechnicalSnapshot";
import { toCommandError } from "@/lib/tauri/command-error";

import {
  acceptSnapshot,
  type TechnicalPageState,
} from "./technical-view-model";

export function useTechnicalState() {
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
      error: toCommandError(error),
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
    await runAction("refresh", () => technicalClient.getSnapshot());
  }, [runAction]);

  const reset = useCallback(async () => {
    await runAction("reset", () => technicalClient.resetSession());
  }, [runAction]);

  const setPassthrough = useCallback(
    async (enabled: boolean) => {
      await runAction("passthrough", () =>
        technicalClient.setPassthrough(enabled),
      );
    },
    [runAction],
  );

  const setAlwaysOnTop = useCallback(
    async (enabled: boolean) => {
      await runAction("always-on-top", () =>
        technicalClient.setAlwaysOnTop(enabled),
      );
    },
    [runAction],
  );

  const setHudModuleVisibility = useCallback(
    async (module: HudModule, visible: boolean) => {
      await runAction("hud-modules", () =>
        technicalClient.setModuleVisibility(module, visible),
      );
    },
    [runAction],
  );

  const moveHudModule = useCallback(
    async (dragged: HudModule, target: HudModule, insertAfter: boolean) => {
      await runAction("module-order", () =>
        technicalClient.moveModule(dragged, target, insertAfter),
      );
    },
    [runAction],
  );

  const setHudWidth = useCallback(
    async (width: number) => {
      await runAction("hud-width", () => technicalClient.setWidth(width));
    },
    [runAction],
  );

  const startCapture = useCallback(async () => {
    await runAction("start-capture", () => technicalClient.startCapture());
  }, [runAction]);

  const stopCapture = useCallback(async () => {
    await runAction("stop-capture", () => technicalClient.stopCapture());
  }, [runAction]);

  const clearActionNotice = useCallback(() => setActionNotice(null), []);

  useEffect(() => {
    void technicalClient.getSnapshot().then(applySnapshot).catch(applyError);
    const unsubscribe = technicalClient.subscribe(applySnapshot, applyError);

    return () => {
      void unsubscribe().catch((error: unknown) => {
        console.error("technical subscription cleanup failed", error);
      });
    };
  }, [applyError, applySnapshot]);

  return {
    state,
    actionNotice,
    clearActionNotice,
    refresh,
    reset,
    setPassthrough,
    setAlwaysOnTop,
    setHudModuleVisibility,
    moveHudModule,
    setHudWidth,
    startCapture,
    stopCapture,
  };
}
