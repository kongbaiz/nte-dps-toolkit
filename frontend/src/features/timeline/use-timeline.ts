import { useCallback, useEffect, useRef, useState } from "react";

import { shouldAcceptDecimalVersion } from "@/lib/decimal-string";
import { timelineClient } from "@/lib/tauri/timeline-client";
import type { TimelineDpsViewMode } from "@/lib/tauri/generated/TimelineDpsViewMode";
import type { TimelineScope } from "@/lib/tauri/generated/TimelineScope";
import type { TimelineSnapshot } from "@/lib/tauri/generated/TimelineSnapshot";
import { type CommandError, toCommandError } from "@/lib/tauri/command-error";

export type TimelinePageState =
  | { status: "loading" }
  | { status: "error"; error: CommandError }
  | { status: "ready"; snapshot: TimelineSnapshot };

export function useTimeline() {
  const [scope, setScope] = useState<TimelineScope>("all");
  const [state, setState] = useState<TimelinePageState>({ status: "loading" });
  const [pending, setPending] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [mutationError, setMutationError] = useState<CommandError | null>(null);
  const mounted = useRef(true);
  const requestGeneration = useRef(0);
  const acceptedGeneration = useRef<string | null>(null);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    let active = true;
    let streamSnapshotReceived = false;
    const generation = ++requestGeneration.current;
    setState({ status: "loading" });
    setMutationError(null);
    void timelineClient
      .getSnapshot(scope)
      .then((snapshot) => {
        if (
          active &&
          mounted.current &&
          generation === requestGeneration.current &&
          shouldAcceptDecimalVersion(
            acceptedGeneration.current,
            snapshot.generation,
            true,
          )
        ) {
          acceptedGeneration.current = snapshot.generation;
          setState({ status: "ready", snapshot });
        }
      })
      .catch((error: unknown) => {
        if (
          active &&
          mounted.current &&
          generation === requestGeneration.current &&
          !streamSnapshotReceived
        ) {
          setState({ status: "error", error: toCommandError(error) });
        }
      });
    const unsubscribe = timelineClient.subscribe(
      scope,
      (snapshot) => {
        if (
          active &&
          mounted.current &&
          snapshot.scope === scope &&
          shouldAcceptDecimalVersion(
            acceptedGeneration.current,
            snapshot.generation,
          )
        ) {
          streamSnapshotReceived = true;
          acceptedGeneration.current = snapshot.generation;
          setState({ status: "ready", snapshot });
        }
      },
      (error) => {
        if (active && mounted.current) setMutationError(error);
      },
    );
    return () => {
      active = false;
      void unsubscribe();
    };
  }, [reloadKey, scope]);

  const retry = useCallback(() => {
    setState({ status: "loading" });
    setReloadKey((value) => value + 1);
  }, []);

  const setPreferences = useCallback(
    async (bucketSeconds: number, viewMode: TimelineDpsViewMode) => {
      if (pending) return;
      setPending(true);
      setMutationError(null);
      try {
        const snapshot = await timelineClient.setPreferences(
          scope,
          bucketSeconds,
          viewMode,
        );
        if (mounted.current && snapshot.scope === scope) {
          requestGeneration.current += 1;
          if (
            shouldAcceptDecimalVersion(
              acceptedGeneration.current,
              snapshot.generation,
              true,
            )
          ) {
            acceptedGeneration.current = snapshot.generation;
            setState({ status: "ready", snapshot });
          }
        }
      } catch (error) {
        if (mounted.current) setMutationError(toCommandError(error));
      } finally {
        if (mounted.current) setPending(false);
      }
    },
    [pending, scope],
  );

  return {
    state,
    scope,
    setScope,
    pending,
    mutationError,
    retry,
    setPreferences,
  };
}
