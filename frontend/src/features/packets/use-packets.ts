import { useCallback, useEffect, useRef, useState } from "react";

import { packetsClient } from "@/lib/tauri/packets-client";
import type { PacketsEvent } from "@/lib/tauri/generated/PacketsEvent";
import type { PacketsSnapshot } from "@/lib/tauri/generated/PacketsSnapshot";
import { type CommandError, toCommandError } from "@/lib/tauri/command-error";

import { mergePacketsEvent } from "./packets-model";

export type PacketsPageState =
  | { status: "loading" }
  | { status: "error"; error: CommandError }
  | { status: "ready"; snapshot: PacketsSnapshot };

export function usePackets() {
  const [state, setState] = useState<PacketsPageState>({ status: "loading" });
  const [streamError, setStreamError] = useState<CommandError | null>(null);
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
    void packetsClient
      .getSnapshot()
      .then((snapshot) => accept({ event: "snapshot", payload: snapshot }))
      .catch((error: unknown) => {
        if (active && snapshotRef.current === null) {
          setState({ status: "error", error: toCommandError(error) });
        }
      });
    const unsubscribe = packetsClient.subscribe(accept, (error) => {
      if (active) setStreamError(error);
    });
    return () => {
      active = false;
      void unsubscribe();
    };
  }, [reloadKey]);

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
