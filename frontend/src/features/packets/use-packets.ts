import { useCallback, useEffect, useRef, useState } from "react";

import { packetsClient, type PacketsClient } from "@/lib/tauri/packets-client";
import {
  packetsError,
  type PacketsCommandError,
  type PacketsEvent,
  type PacketsSnapshot,
} from "@/lib/tauri/packets-contract";

import { mergePacketsEvent } from "./packets-model";

export type PacketsPageState =
  | { status: "loading" }
  | { status: "error"; error: PacketsCommandError }
  | { status: "ready"; snapshot: PacketsSnapshot };

export function usePackets(client: PacketsClient = packetsClient) {
  const [state, setState] = useState<PacketsPageState>({ status: "loading" });
  const [streamError, setStreamError] = useState<PacketsCommandError | null>(
    null,
  );
  const [reloadKey, setReloadKey] = useState(0);
  const snapshotRef = useRef<PacketsSnapshot | null>(null);

  useEffect(() => {
    let active = true;
    snapshotRef.current = null;
    setState({ status: "loading" });
    setStreamError(null);

    const accept = (event: PacketsEvent) => {
      if (!active) return;
      const snapshot = mergePacketsEvent(snapshotRef.current, event);
      snapshotRef.current = snapshot;
      setState({ status: "ready", snapshot });
    };
    void client
      .getSnapshot()
      .then((snapshot) => accept({ mode: "replace", snapshot }))
      .catch((error: unknown) => {
        if (active && snapshotRef.current === null) {
          setState({ status: "error", error: packetsError(error) });
        }
      });
    const unsubscribe = client.subscribe(accept, (error) => {
      if (active) setStreamError(error);
    });
    return () => {
      active = false;
      void unsubscribe();
    };
  }, [client, reloadKey]);

  const retry = useCallback(() => {
    setReloadKey((value) => value + 1);
  }, []);

  return {
    state,
    streamError,
    clearStreamError: () => setStreamError(null),
    retry,
  };
}
