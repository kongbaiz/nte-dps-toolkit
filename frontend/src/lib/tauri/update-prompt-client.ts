import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  parseUpdatePromptSnapshot,
  type UpdatePromptSnapshot,
} from "./update-prompt-contract";

export const UPDATE_AVAILABLE_EVENT = "update-available";

interface UpdatePromptDependencies {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  listen<T>(
    event: string,
    handler: (event: { payload: T }) => void,
  ): Promise<UnlistenFn>;
}

export function createUpdatePromptClient(
  dependencies: UpdatePromptDependencies = { invoke, listen },
) {
  const snapshot = async (
    command: string,
    arguments_?: Record<string, unknown>,
  ) =>
    parseUpdatePromptSnapshot(await dependencies.invoke(command, arguments_));

  return {
    get: (): Promise<UpdatePromptSnapshot> =>
      snapshot("get_main_dps_update_prompt"),
    download: (component: "app" | "mods-plugin") =>
      snapshot("download_main_dps_update", { component }),
    install: () => snapshot("install_main_dps_update"),
    subscribeAvailable: (onAvailable: () => void): Promise<UnlistenFn> =>
      dependencies.listen(UPDATE_AVAILABLE_EVENT, () => onAvailable()),
  };
}

export const updatePromptClient = createUpdatePromptClient();
