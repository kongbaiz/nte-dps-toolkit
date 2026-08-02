import { useCallback, useEffect, useRef, useState } from "react";

import {
  resourcesClient,
  type ResourcesClient,
} from "@/lib/tauri/resources-client";
import {
  resourcesError,
  type ResourcesCommandError,
  type ResourcesSnapshot,
} from "@/lib/tauri/resources-contract";

export type ResourcesPageState =
  | { status: "loading" }
  | { status: "error"; error: ResourcesCommandError }
  | { status: "ready"; snapshot: ResourcesSnapshot };

export function useResources(client: ResourcesClient = resourcesClient) {
  const [state, setState] = useState<ResourcesPageState>({ status: "loading" });
  const [refreshing, setRefreshing] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const snapshotRef = useRef<ResourcesSnapshot | null>(null);
  const completedReloadKeyRef = useRef<number | null>(null);

  useEffect(() => {
    if (
      snapshotRef.current !== null &&
      completedReloadKeyRef.current === reloadKey
    ) {
      return;
    }
    let active = true;
    if (snapshotRef.current === null) {
      setState({ status: "loading" });
    } else {
      setRefreshing(true);
    }
    void client
      .getSnapshot()
      .then((snapshot) => {
        if (!active) return;
        snapshotRef.current = snapshot;
        completedReloadKeyRef.current = reloadKey;
        setState({ status: "ready", snapshot });
      })
      .catch((error: unknown) => {
        if (!active) return;
        if (snapshotRef.current === null) {
          setState({ status: "error", error: resourcesError(error) });
        }
      })
      .finally(() => {
        if (active) setRefreshing(false);
      });
    return () => {
      active = false;
    };
  }, [client, reloadKey]);

  const refresh = useCallback(() => {
    setReloadKey((value) => value + 1);
  }, []);

  return { state, refreshing, refresh };
}
