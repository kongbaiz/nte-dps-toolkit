import { listen } from "@tauri-apps/api/event";

import { invokeCommand } from "@/lib/tauri/command-error";
import type { UpdatePromptSnapshot } from "@/lib/tauri/generated/UpdatePromptSnapshot";

const UPDATE_AVAILABLE_EVENT = "update-available";

export const updatePromptClient = {
  get: () => invokeCommand<UpdatePromptSnapshot>("get_main_dps_update_prompt"),
  download: (component: "app" | "mods-plugin") =>
    invokeCommand<UpdatePromptSnapshot>("download_main_dps_update", {
      component,
    }),
  install: () => invokeCommand<UpdatePromptSnapshot>("install_main_dps_update"),
  subscribeAvailable: (onAvailable: () => void) =>
    listen(UPDATE_AVAILABLE_EVENT, () => onAvailable()),
};
