import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

import {
  parseIslandSnapshot,
  type IslandSnapshot,
} from "@/lib/tauri/island-contract";

const CHANGED_EVENT = "notification-island-changed";

const snapshot = async (
  command: string,
  arguments_?: Record<string, unknown>,
) => parseIslandSnapshot(await invoke<unknown>(command, arguments_));

export const islandClient = {
  getSnapshot: () => snapshot("get_island_snapshot"),
  dismiss: (noticeId: string) =>
    snapshot("dismiss_island_notice", { noticeId }),
  undo: (noticeId: string) => snapshot("undo_island_notice", { noticeId }),
  subscribe(onChanged: () => void) {
    return listen(CHANGED_EVENT, onChanged);
  },
};

export type { IslandSnapshot };
