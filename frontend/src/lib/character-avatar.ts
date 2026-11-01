import { create } from "zustand";

import { characterDataClient } from "@/lib/tauri/character-data-client";
import type { CharacterDataRecordSnapshot } from "@/lib/tauri/generated/CharacterDataRecordSnapshot";

const characterImages = import.meta.glob<string>(
  "@res/images/characters/player*.png",
  {
    eager: true,
    query: "?url",
    import: "default",
  },
);

const imageByFileName = new Map(
  Object.entries(characterImages).map(([path, url]) => [fileName(path), url]),
);

/** Runtime avatar paths by character id, replaced whenever Rust reloads them. */
export const useCharacterAvatarCatalog = create<ReadonlyMap<number, string>>(
  () => new Map(),
);

/** Resolve an avatar from the latest runtime catalog. */
export function resolveCharacterAvatar(charId: number): string | null {
  const avatar = useCharacterAvatarCatalog.getState().get(charId);
  return avatar ? characterAvatarPathUrl(avatar) : null;
}

export async function bootstrapCharacterAvatarCatalog(): Promise<void> {
  const snapshot = await characterDataClient.getSnapshot();
  replaceCharacterAvatarCatalog(snapshot.records);
}

export function replaceCharacterAvatarCatalog(
  records: readonly Pick<CharacterDataRecordSnapshot, "id" | "avatar">[],
): void {
  useCharacterAvatarCatalog.setState(
    new Map(
      records
        .map((record) => [record.id, record.avatar.trim()] as const)
        .filter((entry) => entry[1] !== ""),
    ),
    true,
  );
}

export function characterAvatarPathUrl(avatar: string): string | null {
  return imageByFileName.get(fileName(avatar.trim())) ?? null;
}

function fileName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1).toLowerCase();
}
