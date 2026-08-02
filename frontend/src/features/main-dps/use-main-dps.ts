import { useCallback, useEffect, useRef, useState } from "react";

import { mainDpsClient } from "@/lib/tauri/main-dps-client";
import {
  parseMainDpsCommandError,
  type MainDpsCommandError,
  type MainDpsSnapshot,
} from "@/lib/tauri/main-dps-contract";

import { isGenerationNewer } from "./main-dps-model";

export interface MainDpsActionNotice {
  id: number;
  action: string;
  status: "pending" | "success";
}

export function useMainDps() {
  const [snapshot, setSnapshot] = useState<MainDpsSnapshot | null>(null);
  const [error, setError] = useState<MainDpsCommandError | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [actionNotice, setActionNotice] = useState<MainDpsActionNotice | null>(
    null,
  );
  const generation = useRef<string | null>(null);
  const mounted = useRef(true);
  const actionId = useRef(0);

  const apply = useCallback((next: MainDpsSnapshot) => {
    if (!isGenerationNewer(next.generation, generation.current)) return;
    generation.current = next.generation;
    setSnapshot(next);
  }, []);

  useEffect(() => {
    mounted.current = true;
    void mainDpsClient
      .getSnapshot()
      .then(apply)
      .catch((value) => setError(parseMainDpsCommandError(value)));
    const unsubscribe = mainDpsClient.subscribe(apply, setError);
    return () => {
      mounted.current = false;
      void unsubscribe();
    };
  }, [apply]);

  useEffect(() => {
    if (actionNotice?.status !== "success") return;
    const timer = window.setTimeout(() => setActionNotice(null), 2_200);
    return () => window.clearTimeout(timer);
  }, [actionNotice]);

  const beginAction = useCallback((action: string) => {
    const id = ++actionId.current;
    setActionNotice({ id, action, status: "pending" });
    return id;
  }, []);

  const completeAction = useCallback((id: number, action: string) => {
    if (!mounted.current) return;
    setActionNotice({ id, action, status: "success" });
  }, []);

  const mutate = useCallback(
    async (name: string, action: () => Promise<MainDpsSnapshot>) => {
      if (pending !== null) return;
      const id = beginAction(name);
      setPending(name);
      setError(null);
      try {
        apply(await action());
        completeAction(id, name);
      } catch (value) {
        if (mounted.current) {
          setActionNotice(null);
          setError(parseMainDpsCommandError(value));
        }
      } finally {
        if (mounted.current) setPending(null);
      }
    },
    [apply, beginAction, completeAction, pending],
  );

  const effect = useCallback(
    async (name: string, action: () => Promise<unknown>) => {
      if (pending !== null) return;
      const id = beginAction(name);
      setPending(name);
      setError(null);
      try {
        await action();
        completeAction(id, name);
      } catch (value) {
        if (mounted.current) {
          setActionNotice(null);
          setError(parseMainDpsCommandError(value));
        }
      } finally {
        if (mounted.current) setPending(null);
      }
    },
    [beginAction, completeAction, pending],
  );

  return {
    snapshot,
    error,
    pending,
    actionNotice,
    setActionNotice,
    setError,
    apply,
    mutate,
    effect,
  };
}
