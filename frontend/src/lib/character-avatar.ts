import { characterDataClient } from "@/lib/tauri/character-data-client";
import type { CharacterDataRecord } from "@/lib/tauri/character-data-contract";

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

let avatarByCharacterId = new Map<number, string>();

export async function bootstrapCharacterAvatarCatalog(): Promise<void> {
  const snapshot = await characterDataClient.getSnapshot();
  replaceCharacterAvatarCatalog(snapshot.records);
}

export function replaceCharacterAvatarCatalog(
  records: readonly Pick<CharacterDataRecord, "id" | "avatar">[],
): void {
  avatarByCharacterId = new Map(
    records
      .map((record) => [record.id, record.avatar.trim()] as const)
      .filter((entry) => entry[1] !== ""),
  );
}

export function characterAvatarUrl(charId: number): string | null {
  const avatar = avatarByCharacterId.get(charId);
  return avatar ? characterAvatarPathUrl(avatar) : null;
}

export function characterAvatarPathUrl(avatar: string): string | null {
  return imageByFileName.get(fileName(avatar.trim())) ?? null;
}

function fileName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1).toLowerCase();
}
