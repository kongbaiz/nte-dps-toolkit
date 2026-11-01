import { invokeCommand } from "@/lib/tauri/command-error";
import type { CharacterDataSnapshot } from "@/lib/tauri/generated/CharacterDataSnapshot";

export const characterDataClient = {
  getSnapshot: () =>
    invokeCommand<CharacterDataSnapshot>("get_character_data_snapshot"),
};
