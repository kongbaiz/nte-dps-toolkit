import { useSyncExternalStore } from "react";

import {
  getCharacterAvatarCatalogRevision,
  resolveCharacterAvatar,
  subscribeCharacterAvatarCatalog,
} from "@/lib/character-avatar";

/** Subscribe a React component to runtime avatar catalog replacements. */
export function useCharacterAvatarCatalog(): number {
  return useSyncExternalStore(
    subscribeCharacterAvatarCatalog,
    getCharacterAvatarCatalogRevision,
    getCharacterAvatarCatalogRevision,
  );
}

/** Resolve one avatar and rerender when the runtime catalog changes. */
export function useCharacterAvatar(charId: number | null): string | null {
  useCharacterAvatarCatalog();
  return charId === null ? null : resolveCharacterAvatar(charId);
}
