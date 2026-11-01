import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { invokeCommand, type CommandError } from "@/lib/tauri/command-error";
import type { HitSnapshotReply } from "@/lib/tauri/generated/HitSnapshotReply";
import type { MainDpsDetailColumns } from "@/lib/tauri/generated/MainDpsDetailColumns";
import type { MainDpsDetailSnapshot } from "@/lib/tauri/generated/MainDpsDetailSnapshot";
import { subscribeStream } from "@/lib/tauri/stream-client";

export type MainDpsDetailFilter =
  | "all"
  | "outgoing"
  | "incoming"
  | "characterAttributed"
  | "characterDirect"
  | "reactionDamage"
  | "sharedMechanics"
  | "unattributed"
  | "qteType";

export const mainDpsDetailClient = {
  getHitSnapshot: async (hitId: string, snapshotKey: string) =>
    (
      await invokeCommand<HitSnapshotReply>("get_main_dps_hit_snapshot", {
        hitId,
        snapshotKey,
      })
    ).snapshot,
  getSnapshot: (offset = 0, limit = 200) =>
    invokeCommand<MainDpsDetailSnapshot>("get_main_dps_detail_snapshot", {
      offset,
      limit,
    }),
  setView: (
    filter: MainDpsDetailFilter,
    qteType: string | null,
    skillFilter: string | null,
  ) =>
    invokeCommand<MainDpsDetailSnapshot>("set_main_dps_detail_view", {
      filter,
      qteType,
      skillFilter,
    }),
  setColumns: (columns: MainDpsDetailColumns) =>
    invokeCommand<MainDpsDetailSnapshot>("set_main_dps_detail_columns", {
      columns,
    }),
  startCapture: () =>
    invokeCommand<unknown>("start_main_dps_capture", { replaceCurrent: false }),
  importReplay: () =>
    invokeCommand<unknown>("import_main_dps_replay", {
      kind: "json",
      replaceCurrent: false,
    }),
  subscribeRequested: (onRequested: () => void): Promise<UnlistenFn> =>
    listen("main-dps-detail-requested", onRequested),
  subscribe(
    onSnapshot: (snapshot: MainDpsDetailSnapshot) => void,
    onError: (error: CommandError) => void,
  ): () => void {
    const close = subscribeStream<MainDpsDetailSnapshot>({
      streamKind: "mainDpsDetail",
      subscribeCommand: "subscribe_main_dps_detail",
      unsubscribeCommand: "unsubscribe_main_dps_detail",
      onEvent: onSnapshot,
      onError,
    });
    return () => void close().catch(onError);
  },
};
