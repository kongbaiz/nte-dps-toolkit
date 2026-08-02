import { useCallback, useEffect, useRef, useState } from "react";

import { replaceCharacterAvatarCatalog } from "@/lib/character-avatar";
import { characterDataClient } from "@/lib/tauri/character-data-client";
import {
  characterDataError,
  type CharacterDataCommandError,
  type CharacterDataRecordInput,
  type CharacterDataSnapshot,
} from "@/lib/tauri/character-data-contract";

export interface CharacterDataNotice {
  kind: "success" | "error";
  titleKey: string;
  messageKey: string;
  messageArguments: string[];
}

export function useCharacterData() {
  const mounted = useRef(true);
  const [snapshot, setSnapshot] = useState<CharacterDataSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [loadError, setLoadError] = useState<CharacterDataCommandError | null>(
    null,
  );
  const [notice, setNotice] = useState<CharacterDataNotice | null>(null);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const reload = useCallback(async (announce = true) => {
    setLoading(true);
    setLoadError(null);
    try {
      const next = await characterDataClient.getSnapshot();
      if (!mounted.current) return null;
      replaceCharacterAvatarCatalog(next.records);
      setSnapshot(next);
      if (announce) {
        setNotice({
          kind: "success",
          titleKey: "Character Data",
          messageKey: "Reloaded characters.json",
          messageArguments: [],
        });
      }
      return next;
    } catch (error) {
      if (mounted.current) setLoadError(characterDataError(error));
      return null;
    } finally {
      if (mounted.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload(false);
  }, [reload]);

  const saveRecord = useCallback(async (input: CharacterDataRecordInput) => {
    setSaving(true);
    setNotice(null);
    try {
      const next = await characterDataClient.saveRecord(input);
      if (!mounted.current) return null;
      replaceCharacterAvatarCatalog(next.records);
      setSnapshot(next);
      setNotice({
        kind: "success",
        titleKey: "characters.json saved",
        messageKey:
          "ID {} saved and reloaded; the live-capture mapping updates on next startup",
        messageArguments: [input.id.trim()],
      });
      return next;
    } catch (error) {
      if (mounted.current) {
        const parsed = characterDataError(error);
        setNotice({
          kind: "error",
          titleKey: "Character data was not saved",
          messageKey: parsed.messageKey,
          messageArguments: parsed.messageArguments,
        });
      }
      return null;
    } finally {
      if (mounted.current) setSaving(false);
    }
  }, []);

  return {
    snapshot,
    loading,
    saving,
    loadError,
    notice,
    reload,
    saveRecord,
    clearNotice: () => setNotice(null),
    clearLoadError: () => setLoadError(null),
  };
}
