import {
  resolveCharacterAvatar,
  useCharacterAvatarCatalog,
} from "@/lib/character-avatar";

/** Resolve one avatar and rerender when the runtime catalog changes. */
export function useCharacterAvatar(charId: number | null): string | null {
  return useCharacterAvatarCatalog(() =>
    charId === null ? null : resolveCharacterAvatar(charId),
  );
}
