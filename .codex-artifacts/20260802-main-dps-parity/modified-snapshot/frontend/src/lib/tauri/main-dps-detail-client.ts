import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  parseMainDpsDetailSnapshot,
  type MainDpsDetailSnapshot,
} from "@/lib/tauri/main-dps-detail-contract";

const DETAIL_CHANGED_EVENT = "main-dps-detail-requested";

export const mainDpsDetailClient = {
  async getSnapshot(offset = 0, limit = 200): Promise<MainDpsDetailSnapshot> {
    return parseMainDpsDetailSnapshot(
      await invoke<unknown>("get_main_dps_detail_snapshot", { offset, limit }),
    );
  },
  subscribeRequested(onRequested: () => void): Promise<UnlistenFn> {
    return listen(DETAIL_CHANGED_EVENT, onRequested);
  },
};
