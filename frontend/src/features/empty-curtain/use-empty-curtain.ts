import { useCallback, useEffect, useRef, useState } from "react";

import {
  emptyCurtainClient,
  type EmptyCurtainClient,
  type ManageItemInput,
} from "@/lib/tauri/empty-curtain-client";
import {
  emptyCurtainError,
  type CharacterEquipmentAction,
  type EmptyCurtainCommandError,
  type EmptyCurtainFileResult,
  type EmptyCurtainSnapshot,
  type ItemUid,
} from "@/lib/tauri/empty-curtain-contract";

export type EmptyCurtainPageState =
  | { status: "loading" }
  | { status: "error"; error: EmptyCurtainCommandError }
  | { status: "ready"; snapshot: EmptyCurtainSnapshot };

export function useEmptyCurtain(
  client: EmptyCurtainClient = emptyCurtainClient,
) {
  const [state, setState] = useState<EmptyCurtainPageState>({
    status: "loading",
  });
  const [notice, setNotice] = useState<EmptyCurtainCommandError | null>(null);
  const [actionPending, setActionPending] = useState(false);
  const [refreshing, setRefreshing] = useState(true);
  const [reloadKey, setReloadKey] = useState(0);
  const mounted = useRef(true);
  const requestGeneration = useRef(0);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    let active = true;
    const generation = ++requestGeneration.current;
    setRefreshing(true);
    setState((current) =>
      current.status === "ready" ? current : { status: "loading" },
    );
    setNotice(null);
    void client
      .getSnapshot()
      .then((snapshot) => {
        if (
          active &&
          mounted.current &&
          generation === requestGeneration.current
        ) {
          setState({ status: "ready", snapshot });
        }
      })
      .catch((error: unknown) => {
        if (
          active &&
          mounted.current &&
          generation === requestGeneration.current
        ) {
          setState((current) =>
            current.status === "ready"
              ? current
              : { status: "error", error: emptyCurtainError(error) },
          );
          setNotice(emptyCurtainError(error));
        }
      })
      .finally(() => {
        if (
          active &&
          mounted.current &&
          generation === requestGeneration.current
        )
          setRefreshing(false);
      });
    return () => {
      active = false;
    };
  }, [client, reloadKey]);

  useEffect(() => {
    let active = true;
    const unsubscribe = client.subscribe(
      (snapshot) => {
        if (!active || !mounted.current) return;
        setState({ status: "ready", snapshot });
        if (!snapshot.canOperate) {
          setNotice((current) =>
            current &&
            [
              "plugin_unavailable",
              "plugin_busy",
              "plugin_unsupported",
              "empty_curtain_operation_failed",
            ].includes(current.code)
              ? null
              : current,
          );
        } else if (snapshot.operation.status === "error") {
          setNotice((current) => {
            const argumentsMatch =
              current?.messageArguments.join("\u0000") ===
              snapshot.operation.messageArguments.join("\u0000");
            if (
              current?.messageKey === snapshot.operation.messageKey &&
              argumentsMatch
            ) {
              return current;
            }
            return {
              code: "empty_curtain_operation_failed",
              messageKey: snapshot.operation.messageKey,
              messageArguments: snapshot.operation.messageArguments,
            };
          });
        } else {
          // A recovered background read must remove its own old error toast,
          // without dismissing unrelated command/file-dialog errors.
          setNotice((current) =>
            current?.code === "empty_curtain_operation_failed" ? null : current,
          );
        }
      },
      (error) => {
        if (active && mounted.current) setNotice(error);
      },
    );
    return () => {
      active = false;
      void unsubscribe();
    };
  }, [client]);

  const runSnapshotAction = useCallback(
    async (operation: Promise<EmptyCurtainSnapshot>) => {
      setActionPending(true);
      setNotice(null);
      try {
        const snapshot = await operation;
        if (mounted.current) setState({ status: "ready", snapshot });
      } catch (error) {
        if (mounted.current) setNotice(emptyCurtainError(error));
      } finally {
        if (mounted.current) setActionPending(false);
      }
    },
    [],
  );

  const runFileAction = useCallback(
    async (operation: Promise<EmptyCurtainFileResult>) => {
      setActionPending(true);
      setNotice(null);
      try {
        const result = await operation;
        if (mounted.current)
          setState({ status: "ready", snapshot: result.snapshot });
      } catch (error) {
        if (mounted.current) setNotice(emptyCurtainError(error));
      } finally {
        if (mounted.current) setActionPending(false);
      }
    },
    [],
  );

  const positions = useCallback(
    async (item: ItemUid, character: ItemUid) => {
      try {
        return await client.positions(item, character);
      } catch (error) {
        if (mounted.current) setNotice(emptyCurtainError(error));
        return [];
      }
    },
    [client],
  );

  const canOperate = state.status === "ready" && state.snapshot.canOperate;

  return {
    state,
    notice,
    actionPending:
      actionPending ||
      (state.status === "ready" &&
        state.snapshot.operation.status === "pending"),
    refreshing,
    clearNotice: () => setNotice(null),
    retry: () => {
      if (state.status === "ready" && !canOperate) return;
      setRefreshing(true);
      setReloadKey((value) => value + 1);
    },
    positions,
    manageItem: (input: ManageItemInput) =>
      canOperate
        ? runSnapshotAction(client.manageItem(input))
        : Promise.resolve(),
    characterAction: (character: ItemUid, action: CharacterEquipmentAction) =>
      canOperate
        ? runSnapshotAction(client.characterAction(character, action))
        : Promise.resolve(),
    exportInventory: () => runFileAction(client.exportInventory()),
    exportLoadout: (character: ItemUid) =>
      runFileAction(client.exportLoadout(character)),
    importLoadout: () =>
      canOperate ? runFileAction(client.importLoadout()) : Promise.resolve(),
  };
}
