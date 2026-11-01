import { useCallback, useEffect, useRef, useState } from "react";

import { shouldAcceptDecimalVersion } from "@/lib/decimal-string";
import { historyClient } from "@/lib/tauri/history-client";
import type { AbyssHalf } from "@/lib/tauri/generated/AbyssHalf";
import type { HistoryComparisonSnapshot } from "@/lib/tauri/generated/HistoryComparisonSnapshot";
import type { HistoryExportSnapshot } from "@/lib/tauri/generated/HistoryExportSnapshot";
import type { HistorySnapshot } from "@/lib/tauri/generated/HistorySnapshot";
import { type CommandError, toCommandError } from "@/lib/tauri/command-error";

import type { HistoryPageState } from "./history-view-model";

export function useHistory() {
  const [state, setState] = useState<HistoryPageState>({ status: "loading" });
  const [pendingAction, setPendingAction] = useState<string | null>(null);
  const [mutationError, setMutationError] = useState<CommandError | null>(null);
  const [comparison, setComparison] =
    useState<HistoryComparisonSnapshot | null>(null);
  const [statusMessageKey, setStatusMessageKey] = useState<string | null>(null);
  const [preferredRecordId, setPreferredRecordId] = useState<string | null>(
    null,
  );
  const [undoDeletion, setUndoDeletion] = useState<{
    token: string;
    expiresAt: number;
  } | null>(null);
  const mounted = useRef(true);
  const requestGeneration = useRef(0);
  const operationPending = useRef(false);
  const comparisonGeneration = useRef(0);
  const acceptedRevision = useRef<string | null>(null);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const applySnapshot = useCallback(
    (snapshot: HistorySnapshot, allowEqual = false): boolean => {
      if (
        !shouldAcceptDecimalVersion(
          acceptedRevision.current,
          snapshot.revision,
          allowEqual,
        )
      ) {
        return false;
      }
      acceptedRevision.current = snapshot.revision;
      setState({ status: "ready", snapshot });
      return true;
    },
    [],
  );
  const clearPreferredRecordId = useCallback(() => {
    setPreferredRecordId(null);
  }, []);
  const clearComparison = useCallback(() => {
    comparisonGeneration.current += 1;
    setComparison(null);
  }, []);

  const refresh = useCallback(async () => {
    if (operationPending.current) return false;
    operationPending.current = true;
    setPendingAction("reload");
    const generation = ++requestGeneration.current;
    setMutationError(null);
    try {
      const snapshot = await historyClient.getSnapshot();
      if (mounted.current && generation === requestGeneration.current) {
        return applySnapshot(snapshot, true);
      }
      return false;
    } catch (error) {
      if (!mounted.current || generation !== requestGeneration.current) return;
      const parsed = toCommandError(error);
      setState((current) =>
        current.status === "ready"
          ? current
          : { status: "error", messageKey: parsed.messageKey },
      );
      setMutationError(parsed);
      return false;
    } finally {
      operationPending.current = false;
      if (mounted.current) setPendingAction(null);
    }
  }, [applySnapshot]);

  const reload = useCallback(async () => {
    if ((await refresh()) && mounted.current) {
      setStatusMessageKey("History list refreshed");
    }
  }, [refresh]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const unsubscribe = historyClient.subscribe(
      (snapshot) => {
        if (mounted.current) applySnapshot(snapshot);
      },
      (error) => {
        if (mounted.current) setMutationError(error);
      },
    );
    return () => void unsubscribe();
  }, [applySnapshot]);

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
      if (operationPending.current) return false;
      operationPending.current = true;
      setPendingAction(action);
      setMutationError(null);
      try {
        const snapshot = await command();
        if (mounted.current) {
          requestGeneration.current += 1;
          return applySnapshot(snapshot, true);
        }
        return false;
      } catch (error) {
        if (mounted.current) setMutationError(toCommandError(error));
        return false;
      } finally {
        operationPending.current = false;
        if (mounted.current) setPendingAction(null);
      }
    },
    [applySnapshot],
  );

  const compare = useCallback(async (leftId: string, rightId: string) => {
    if (operationPending.current) return;
    operationPending.current = true;
    setPendingAction("compare");
    setMutationError(null);
    const generation = ++comparisonGeneration.current;
    try {
      const result = await historyClient.compare(leftId, rightId);
      if (mounted.current && generation === comparisonGeneration.current) {
        setComparison(result);
      }
    } catch (error) {
      if (mounted.current) setMutationError(toCommandError(error));
    } finally {
      operationPending.current = false;
      if (mounted.current) setPendingAction(null);
    }
  }, []);

  const exportRecord = useCallback(
    async (recordId: string): Promise<HistoryExportSnapshot | null> => {
      if (operationPending.current) return null;
      operationPending.current = true;
      setPendingAction("export");
      setMutationError(null);
      try {
        return await historyClient.exportRecord(recordId);
      } catch (error) {
        if (mounted.current) setMutationError(toCommandError(error));
        return null;
      } finally {
        operationPending.current = false;
        if (mounted.current) setPendingAction(null);
      }
    },
    [],
  );

  const importFile = useCallback(async () => {
    if (operationPending.current) return;
    operationPending.current = true;
    setPendingAction("import");
    setMutationError(null);
    try {
      const result = await historyClient.importFile();
      if (mounted.current && result.performed) {
        requestGeneration.current += 1;
        applySnapshot(result.history, true);
        setPreferredRecordId(result.importedRecordId);
        setComparison(null);
        setStatusMessageKey("History summary imported");
      }
    } catch (error) {
      if (mounted.current) setMutationError(toCommandError(error));
    } finally {
      operationPending.current = false;
      if (mounted.current) setPendingAction(null);
    }
  }, [applySnapshot]);

  const exportRecordFile = useCallback(async (recordId: string) => {
    if (operationPending.current) return;
    operationPending.current = true;
    setPendingAction("export");
    setMutationError(null);
    try {
      const result = await historyClient.exportRecordFile(recordId);
      if (mounted.current && result.performed) {
        setStatusMessageKey("History summary exported");
      }
    } catch (error) {
      if (mounted.current) setMutationError(toCommandError(error));
    } finally {
      operationPending.current = false;
      if (mounted.current) setPendingAction(null);
    }
  }, []);

  const deleteRecord = useCallback(
    async (recordId: string) => {
      if (operationPending.current) return;
      operationPending.current = true;
      setPendingAction("delete");
      setMutationError(null);
      try {
        const result = await historyClient.deleteRecord(recordId);
        if (mounted.current) {
          requestGeneration.current += 1;
          applySnapshot(result.history, true);
          setComparison(null);
          setUndoDeletion({
            token: result.undoToken,
            expiresAt: Date.now() + result.undoExpiresMs,
          });
          setStatusMessageKey("History summary deleted");
        }
      } catch (error) {
        if (mounted.current) setMutationError(toCommandError(error));
      } finally {
        operationPending.current = false;
        if (mounted.current) setPendingAction(null);
      }
    },
    [applySnapshot],
  );

  const restoreDeleted = useCallback(async () => {
    if (operationPending.current || undoDeletion === null) return;
    operationPending.current = true;
    setPendingAction("restore");
    setMutationError(null);
    try {
      const snapshot = await historyClient.restoreDeleted(undoDeletion.token);
      if (mounted.current) {
        applySnapshot(snapshot, true);
        setUndoDeletion(null);
        setStatusMessageKey("History summary restored");
      }
    } catch (error) {
      if (mounted.current) setMutationError(toCommandError(error));
    } finally {
      operationPending.current = false;
      if (mounted.current) setPendingAction(null);
    }
  }, [applySnapshot, undoDeletion]);

  return {
    state,
    pendingAction,
    mutationError,
    comparison,
    statusMessageKey,
    preferredRecordId,
    clearPreferredRecordId,
    clearComparison,
    undoDeletion,
    refresh: reload,
    saveCurrent: async () => {
      if (
        (await mutate("save", () => historyClient.saveCurrent())) &&
        mounted.current
      ) {
        setStatusMessageKey("History summary saved");
      }
    },
    importJson: (json: string) =>
      mutate("import", () => historyClient.importJson(json)),
    importFile,
    deleteRecord,
    restoreDeleted,
    setPrediction: async (recordId: string, line: AbyssHalf) => {
      if (
        (await mutate(`prediction:${line}`, () =>
          historyClient.setPrediction(recordId, line),
        )) &&
        mounted.current
      ) {
        setStatusMessageKey(
          line === "first"
            ? "Set as the upper-line prediction team"
            : "Set as the lower-line prediction team",
        );
      }
    },
    compare,
    exportRecord,
    exportRecordFile,
  };
}
