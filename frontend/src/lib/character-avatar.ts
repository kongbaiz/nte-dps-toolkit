import characterCatalog from "@res/data/characters/characters.json";

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

const avatarByCharacterId = loadAvatarMap(characterCatalog);

export function characterAvatarUrl(charId: number): string | null {
  const avatar = avatarByCharacterId.get(String(charId));
  return avatar ? characterAvatarPathUrl(avatar) : null;
}

export function characterAvatarPathUrl(avatar: string): string | null {
  return imageByFileName.get(fileName(avatar.trim())) ?? null;
}

function loadAvatarMap(value: unknown): Map<string, string> {
  const result = new Map<string, string>();
  if (!isRecord(value) || !isRecord(value.characters)) return result;
  for (const [charId, candidate] of Object.entries(value.characters)) {
    if (isRecord(candidate) && typeof candidate.avatar === "string") {
      result.set(charId, candidate.avatar);
    }
  }
  return result;
}

function fileName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1).toLowerCase();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
