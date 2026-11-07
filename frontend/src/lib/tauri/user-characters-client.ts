import { invokeCommand } from "@/lib/tauri/command-error";
import type { CharacterRequest } from "@/lib/tauri/generated/CharacterRequest";
import type { CharacterResponse } from "@/lib/tauri/generated/CharacterResponse";

export const INITIAL_CHARACTER_REQUEST: CharacterRequest = {
  refresh: false,
  expectedIdentity: null,
  expectedSnapshotId: null,
  offset: 0,
  selectedUid: null,
  query: "",
};

export const userCharactersClient = {
  read: (request: CharacterRequest) =>
    invokeCommand<CharacterResponse>("get_user_characters", { request }),
};
