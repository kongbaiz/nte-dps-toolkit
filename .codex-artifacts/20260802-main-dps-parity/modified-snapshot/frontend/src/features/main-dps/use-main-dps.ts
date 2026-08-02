import { useCallback, useEffect, useRef, useState } from "react";

import { mainDpsClient } from "@/lib/tauri/main-dps-client";
import {
  parseMainDpsCommandError,
  type MainDpsCommandError,
  type MainDpsSnapshot,
} from "@/lib/tauri/main-dps-contract";

import { isGenerationNewer } from "./main-dps-model";

export function useMainDps() {
  const [snapshot, setSnapshot] = useState<MainDpsSnapshot | null>(null);
  const [error, setError] = useState<MainDpsCommandError | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const generation = useRef<string | null>(null);
  const mounted = useRef(true);

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

  const mutate = useCallback(
    async (name: string, action: () => Promise<MainDpsSnapshot>) => {
      if (pending !== null) return;
      setPending(name);
      setError(null);
      try {
        apply(await action());
      } catch (value) {
        if (mounted.current) setError(parseMainDpsCommandError(value));
      } finally {
        if (mounted.current) setPending(null);
      }
    },
    [apply, pending],
  );

  const effect = useCallback(
    async (name: string, action: () => Promise<unknown>) => {
      if (pending !== null) return;
      setPending(name);
      setError(null);
      try {
        await action();
      } catch (value) {
        if (mounted.current) setError(parseMainDpsCommandError(value));
      } finally {
        if (mounted.current) setPending(null);
      }
    },
    [pending],
  );

  return { snapshot, error, pending, setError, apply, mutate, effect };
}
