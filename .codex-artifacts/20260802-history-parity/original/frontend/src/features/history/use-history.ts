import { useCallback, useEffect, useRef, useState } from "react";

import {
  historyClient,
  historyClientError,
  type HistoryClient,
} from "@/lib/tauri/history-client";
import type {
  HistoryCommandError,
  HistoryComparison,
  HistoryExport,
  HistoryLine,
  HistorySnapshot,
} from "@/lib/tauri/history-contract";

import type { HistoryPageState } from "./history-view-model";

export function useHistory(client: HistoryClient = historyClient) {
  const [state, setState] = useState<HistoryPageState>({ status: "loading" });
  const [pendingAction, setPendingAction] = useState<string | null>(null);
  const [mutationError, setMutationError] =
    useState<HistoryCommandError | null>(null);
  const [comparison, setComparison] = useState<HistoryComparison | null>(null);
  const [undoDeletion, setUndoDeletion] = useState<{
    token: string;
    expiresAt: number;
  } | null>(null);
  const mounted = useRef(true);
  const requestGeneration = useRef(0);
  const operationPending = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const applySnapshot = useCallback((snapshot: HistorySnapshot) => {
    setState({ status: "ready", snapshot });
  }, []);

  const refresh = useCallback(async () => {
    if (operationPending.current) return;
    operationPending.current = true;
    setPendingAction("reload");
    const generation = ++requestGeneration.current;
    setMutationError(null);
    try {
      const snapshot = await client.getSnapshot();
      if (mounted.current && generation === requestGeneration.current)
        applySnapshot(snapshot);
    } catch (error) {
      if (!mounted.current || generation !== requestGeneration.current) return;
      const parsed = historyClientError(error);
      setState((current) =>
        current.status === "ready"
          ? current
          : { status: "error", messageKey: parsed.messageKey },
      );
      setMutationError(parsed);
    } finally {
      operationPending.current = false;
      if (mounted.current) setPendingAction(null);
    }
  }, [applySnapshot, client]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const unsubscribe = client.subscribe(
      (snapshot) => {
        if (mounted.current) applySnapshot(snapshot);
      },
      (error) => {
        if (mounted.current) setMutationError(error);
      },
    );
    return () => void unsubscribe();
  }, [applySnapshot, client]);

  useEffect(() => {
    if (undoDeletion === null) return;
    const timeout = window.setTimeout(
      () => setUndoDeletion(null),
      Math.max(0, undoDeletion.expiresAt - Date.now()),
    );
    return () => window.clearTimeout(timeout);
  }, [undoDeletion]);

  const mutate = useCallback(
    async (action: string, command: () => Promise<HistorySnapshot>) => {
      if (operationPending.current) return;
      operationPending.current = true;
      setPendingAction(action);
      setMutationError(null);
      try {
        const snapshot = await command();
        if (mounted.current) {
          requestGeneration.current += 1;
          applySnapshot(snapshot);
          setComparison(null);
        }
      } catch (error) {
        if (mounted.current) setMutationError(historyClientError(error));
      } finally {
        operationPending.current = false;
        if (mounted.current) setPendingAction(null);
      }
    },
    [applySnapshot],
  );

  const compare = useCallback(
    async (leftId: string, rightId: string) => {
      if (operationPending.current) return;
      operationPending.current = true;
      setPendingAction("compare");
      setMutationError(null);
      try {
        const result = await client.compare(leftId, rightId);
        if (mounted.current) setComparison(result);
      } catch (error) {
        if (mounted.current) setMutationError(historyClientError(error));
      } finally {
        operationPending.current = false;
        if (mounted.current) setPendingAction(null);
      }
    },
    [client],
  );

  const exportRecord = useCallback(
    async (recordId: string): Promise<HistoryExport | null> => {
      if (operationPending.current) return null;
      operationPending.current = true;
      setPendingAction("export");
      setMutationError(null);
      try {
        return await client.exportRecord(recordId);
      } catch (error) {
        if (mounted.current) setMutationError(historyClientError(error));
        return null;
      } finally {
        operationPending.current = false;
        if (mounted.current) setPendingAction(null);
      }
    },
    [client],
  );

  const deleteRecord = useCallback(
    async (recordId: string) => {
      if (operationPending.current) return;
      operationPending.current = true;
      setPendingAction("delete");
      setMutationError(null);
      try {
        const result = await client.deleteRecord(recordId);
        if (mounted.current) {
          requestGeneration.current += 1;
          applySnapshot(result.history);
          setComparison(null);
          setUndoDeletion({
            token: result.undoToken,
            expiresAt: Date.now() + result.undoExpiresMs,
          });
        }
      } catch (error) {
        if (mounted.current) setMutationError(historyClientError(error));
      } finally {
        operationPending.current = false;
        if (mounted.current) setPendingAction(null);
      }
    },
    [applySnapshot, client],
  );

  const restoreDeleted = useCallback(async () => {
    if (operationPending.current || undoDeletion === null) return;
    operationPending.current = true;
    setPendingAction("restore");
    setMutationError(null);
    try {
      const snapshot = await client.restoreDeleted(undoDeletion.token);
      if (mounted.current) {
        applySnapshot(snapshot);
        setUndoDeletion(null);
      }
    } catch (error) {
      if (mounted.current) setMutationError(historyClientError(error));
    } finally {
      operationPending.current = false;
      if (mounted.current) setPendingAction(null);
    }
  }, [applySnapshot, client, undoDeletion]);

  return {
    state,
    pendingAction,
    mutationError,
    comparison,
    undoDeletion,
    refresh,
    saveCurrent: () => mutate("save", () => client.saveCurrent()),
    importJson: (json: string) =>
      mutate("import", () => client.importJson(json)),
    deleteRecord,
    restoreDeleted,
    setPrediction: (recordId: string, line: HistoryLine) =>
      mutate(`prediction:${line}`, () => client.setPrediction(recordId, line)),
    compare,
    exportRecord,
  };
}
