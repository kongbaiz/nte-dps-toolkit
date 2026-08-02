import { useCallback, useEffect, useRef, useState } from "react";

import { shouldAcceptDecimalVersion } from "@/lib/decimal-string";
import {
  timelineClient,
  type TimelineClient,
} from "@/lib/tauri/timeline-client";
import {
  timelineError,
  type TimelineCommandError,
  type TimelineCurveMode,
  type TimelineScope,
  type TimelineSnapshot,
} from "@/lib/tauri/timeline-contract";

export type TimelinePageState =
  | { status: "loading" }
  | { status: "error"; error: TimelineCommandError }
  | { status: "ready"; snapshot: TimelineSnapshot };

export function useTimeline(client: TimelineClient = timelineClient) {
  const [scope, setScope] = useState<TimelineScope>("all");
  const [state, setState] = useState<TimelinePageState>({ status: "loading" });
  const [pending, setPending] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [mutationError, setMutationError] =
    useState<TimelineCommandError | null>(null);
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
    void client
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
          setState({ status: "error", error: timelineError(error) });
        }
      });
    const unsubscribe = client.subscribe(
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
  }, [client, reloadKey, scope]);

  const retry = useCallback(() => {
    setState({ status: "loading" });
    setReloadKey((value) => value + 1);
  }, []);

  const setPreferences = useCallback(
    async (bucketSeconds: number, viewMode: TimelineCurveMode) => {
      if (pending) return;
      setPending(true);
      setMutationError(null);
      try {
        const snapshot = await client.setPreferences(
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
        if (mounted.current) setMutationError(timelineError(error));
      } finally {
        if (mounted.current) setPending(false);
      }
    },
    [client, pending, scope],
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
