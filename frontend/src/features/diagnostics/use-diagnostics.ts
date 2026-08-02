import { useCallback, useEffect, useRef, useState } from "react";

import {
  diagnosticsClient,
  type DiagnosticsClient,
} from "@/lib/tauri/diagnostics-client";
import {
  diagnosticsError,
  type DiagnosticsCommandError,
  type DiagnosticsSnapshot,
} from "@/lib/tauri/diagnostics-contract";

import { diagnosticsSnapshotIsAtLeast } from "./diagnostics-model";

export type DiagnosticsAction =
  "run" | "import-pcapng" | "import-json" | "export-json" | "export-pcapng";

export type DiagnosticsPageState =
  | { status: "loading" }
  | { status: "error"; error: DiagnosticsCommandError }
  | { status: "ready"; snapshot: DiagnosticsSnapshot };

export function useDiagnostics(client: DiagnosticsClient = diagnosticsClient) {
  const [state, setState] = useState<DiagnosticsPageState>({
    status: "loading",
  });
  const [streamError, setStreamError] =
    useState<DiagnosticsCommandError | null>(null);
  const [actionError, setActionError] =
    useState<DiagnosticsCommandError | null>(null);
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
    void client
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
          const parsed = diagnosticsError(error);
          if (snapshotRef.current === null) {
            setState({ status: "error", error: parsed });
          } else {
            setStreamError(parsed);
          }
        }
      });
    const unsubscribe = client.subscribe(
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
  }, [client, publishSnapshot, reloadKey]);

  const perform = useCallback(
    async (action: DiagnosticsAction): Promise<boolean> => {
      setPendingAction(action);
      setActionError(null);
      try {
        if (action === "run") {
          const snapshot = await client.run();
          publishSnapshot(snapshot);
          return true;
        }
        const result = await {
          "import-pcapng": client.importPcapng,
          "import-json": client.importJson,
          "export-json": client.exportJson,
          "export-pcapng": client.exportPcapng,
        }[action]();
        publishSnapshot(result.snapshot);
        return result.performed;
      } catch (error) {
        if (mounted.current) setActionError(diagnosticsError(error));
        return false;
      } finally {
        if (mounted.current) setPendingAction(null);
      }
    },
    [client, publishSnapshot],
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
