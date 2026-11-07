import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

import type { IslandSnapshot } from "@/lib/tauri/generated/IslandSnapshot";

const CHANGED_EVENT = "notification-island-changed";

export const islandClient = {
  getSnapshot: () => invoke<IslandSnapshot>("get_island_snapshot"),
  dismiss: (noticeId: string) =>
    invoke<IslandSnapshot>("dismiss_island_notice", { noticeId }),
  undo: (noticeId: string) =>
    invoke<IslandSnapshot>("undo_island_notice", { noticeId }),
  subscribe(onChanged: () => void) {
    return listen(CHANGED_EVENT, onChanged);
  },
};
