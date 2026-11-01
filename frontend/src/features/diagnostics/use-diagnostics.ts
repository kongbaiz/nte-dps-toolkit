import { useCallback, useEffect, useRef, useState } from "react";

import { diagnosticsClient } from "@/lib/tauri/diagnostics-client";
import type { DiagnosticsSnapshot } from "@/lib/tauri/generated/DiagnosticsSnapshot";
import { type CommandError, toCommandError } from "@/lib/tauri/command-error";

import { diagnosticsSnapshotIsAtLeast } from "./diagnostics-model";

export type DiagnosticsAction =
  "run" | "import-pcapng" | "import-json" | "export-json" | "export-pcapng";

export type DiagnosticsPageState =
  | { status: "loading" }
  | { status: "error"; error: CommandError }
  | { status: "ready"; snapshot: DiagnosticsSnapshot };

export function useDiagnostics() {
  const [state, setState] = useState<DiagnosticsPageState>({
    status: "loading",
  });
  const [streamError, setStreamError] = useState<CommandError | null>(null);
  const [actionError, setActionError] = useState<CommandError | null>(null);
  const [pendingAction, setPendingAction] = useState<DiagnosticsAction | null>(
    null,
  );
  const [reloadKey, setReloadKey] = useState(0);
  const mounted = useRef(true);
  const requestGeneration = useRef(0);
  const snapshotRef = useRef<DiagnosticsSnapshot | null>(null);

  const publishSnapshot = useCallback((snapshot: DiagnosticsSnapshot) => {
    if (
      mounted.current &&
      diagnosticsSnapshotIsAtLeast(snapshot, snapshotRef.current)
    ) {
      snapshotRef.current = snapshot;
      setState({ status: "ready", snapshot });
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    let active = true;
    const generation = ++requestGeneration.current;
    if (snapshotRef.current === null) {
      setState({ status: "loading" });
    }
    setStreamError(null);
    void diagnosticsClient
      .getSnapshot()
      .then((snapshot) => {
        if (
          active &&
          mounted.current &&
          generation === requestGeneration.current
        ) {
          publishSnapshot(snapshot);
        }
      })
      .catch((error: unknown) => {
        if (
          active &&
          mounted.current &&
          generation === requestGeneration.current
        ) {
          const parsed = toCommandError(error);
          if (snapshotRef.current === null) {
            setState({ status: "error", error: parsed });
          } else {
            setStreamError(parsed);
          }
        }
      });
    const unsubscribe = diagnosticsClient.subscribe(
      (snapshot) => {
        if (active && mounted.current) {
          setStreamError(null);
          publishSnapshot(snapshot);
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
  }, [publishSnapshot, reloadKey]);

  const perform = useCallback(
    async (action: DiagnosticsAction): Promise<boolean> => {
      setPendingAction(action);
      setActionError(null);
      try {
        if (action === "run") {
          const snapshot = await diagnosticsClient.run();
          publishSnapshot(snapshot);
          return true;
        }
        const result = await {
          "import-pcapng": diagnosticsClient.importPcapng,
          "import-json": diagnosticsClient.importJson,
          "export-json": diagnosticsClient.exportJson,
          "export-pcapng": diagnosticsClient.exportPcapng,
        }[action]();
        publishSnapshot(result.snapshot);
        return result.performed;
      } catch (error) {
        if (mounted.current) setActionError(toCommandError(error));
        return false;
      } finally {
        if (mounted.current) setPendingAction(null);
      }
    },
    [publishSnapshot],
  );

  return {
    state,
    streamError,
    actionError,
    pendingAction,
    perform,
    clearNotice: () => {
      setStreamError(null);
      setActionError(null);
    },
    retry: () => setReloadKey((value) => value + 1),
  };
}
