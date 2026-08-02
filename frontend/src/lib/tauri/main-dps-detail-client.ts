import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  parseMainDpsDetailSnapshot,
  type MainDpsDetailFilter,
  type MainDpsDetailColumns,
  type MainDpsDetailSnapshot,
} from "@/lib/tauri/main-dps-detail-contract";

const DETAIL_CHANGED_EVENT = "main-dps-detail-requested";

export const mainDpsDetailClient = {
  async getSnapshot(offset = 0, limit = 200): Promise<MainDpsDetailSnapshot> {
    return parseMainDpsDetailSnapshot(
      await invoke<unknown>("get_main_dps_detail_snapshot", { offset, limit }),
    );
  },
  async setView(
    filter: MainDpsDetailFilter,
    qteType: string | null,
    skillFilter: string | null,
  ): Promise<MainDpsDetailSnapshot> {
    return parseMainDpsDetailSnapshot(
      await invoke<unknown>("set_main_dps_detail_view", {
        filter,
        qteType,
        skillFilter,
      }),
    );
  },
  async setColumns(
    columns: MainDpsDetailColumns,
  ): Promise<MainDpsDetailSnapshot> {
    return parseMainDpsDetailSnapshot(
      await invoke<unknown>("set_main_dps_detail_columns", { columns }),
    );
  },
  startCapture: () =>
    invoke<unknown>("start_main_dps_capture", { replaceCurrent: false }),
  importReplay: () =>
    invoke<unknown>("import_main_dps_replay", {
      kind: "json",
      replaceCurrent: false,
    }),
  subscribeRequested(onRequested: () => void): Promise<UnlistenFn> {
    return listen(DETAIL_CHANGED_EVENT, onRequested);
  },
  subscribe(
    onSnapshot: (snapshot: MainDpsDetailSnapshot) => void,
    onError: (error: unknown) => void,
  ): () => Promise<void> {
    const subscriptionId = crypto.randomUUID();
    const channel = new Channel<unknown>();
    channel.onmessage = (message) => {
      try {
        onSnapshot(parseMainDpsDetailSnapshot(message));
      } catch (error) {
        onError(error);
      }
    };
    void invoke("subscribe_main_dps_detail", {
      subscriptionId,
      onEvent: channel,
    }).catch(onError);
    return () =>
      invoke("unsubscribe_main_dps_detail", { subscriptionId })
        .then(() => undefined)
        .catch(() => undefined);
  },
};
