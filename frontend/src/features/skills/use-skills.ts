import { useCallback, useEffect, useRef, useState } from "react";

import { shouldAcceptDecimalVersion } from "@/lib/decimal-string";
import { skillsClient } from "@/lib/tauri/skills-client";
import type { SkillsSnapshot } from "@/lib/tauri/generated/SkillsSnapshot";
import type { TimelineScope } from "@/lib/tauri/generated/TimelineScope";
import { type CommandError, toCommandError } from "@/lib/tauri/command-error";

export type SkillsPageState =
  | { status: "loading" }
  | { status: "error"; error: CommandError }
  | { status: "ready"; snapshot: SkillsSnapshot };

export function useSkills() {
  const [scope, setScope] = useState<TimelineScope>("all");
  const [state, setState] = useState<SkillsPageState>({ status: "loading" });
  const [streamError, setStreamError] = useState<CommandError | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
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
    setStreamError(null);
    void skillsClient
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
    const unsubscribe = skillsClient.subscribe(
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
          setStreamError(null);
          setState({ status: "ready", snapshot });
        }
      },
      (error) => {
        if (active && mounted.current) setStreamError(error);
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

  return {
    state,
    scope,
    setScope,
    streamError,
    clearStreamError: () => setStreamError(null),
    retry,
  };
}
