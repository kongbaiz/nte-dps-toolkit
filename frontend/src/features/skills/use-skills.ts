import { useCallback, useEffect, useRef, useState } from "react";

import { shouldAcceptDecimalVersion } from "@/lib/decimal-string";
import { skillsClient, type SkillsClient } from "@/lib/tauri/skills-client";
import {
  skillsError,
  type SkillsCommandError,
  type SkillsScope,
  type SkillsSnapshot,
} from "@/lib/tauri/skills-contract";

export type SkillsPageState =
  | { status: "loading" }
  | { status: "error"; error: SkillsCommandError }
  | { status: "ready"; snapshot: SkillsSnapshot };

export function useSkills(client: SkillsClient = skillsClient) {
  const [scope, setScope] = useState<SkillsScope>("all");
  const [state, setState] = useState<SkillsPageState>({ status: "loading" });
  const [streamError, setStreamError] = useState<SkillsCommandError | null>(
    null,
  );
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
          setState({ status: "error", error: skillsError(error) });
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
  }, [client, reloadKey, scope]);

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
