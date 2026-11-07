import { useCallback, useEffect, useRef, useState } from "react";
import {
  INITIAL_CHARACTER_REQUEST,
  userCharactersClient,
} from "@/lib/tauri/user-characters-client";
import type { CharacterRequest } from "@/lib/tauri/generated/CharacterRequest";
import type { CharacterResponse } from "@/lib/tauri/generated/CharacterResponse";
import { toCommandError } from "@/lib/tauri/command-error";

export function useUserCharacters() {
  const [snapshot, setSnapshot] = useState<CharacterResponse | null>(null);
  const [pending, setPending] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const load = useCallback((request: CharacterRequest) => {
    const token = ++generation.current;
    if (timer.current !== null) clearTimeout(timer.current);
    setPending(true);
    setError(null);
    let polls = 0;
    const read = async (input: CharacterRequest): Promise<void> => {
      try {
        const result = await userCharactersClient.read(input);
        if (token !== generation.current) return;
        setSnapshot(result);
        if (result.state === "queued" || result.state === "reading") {
          if (++polls >= 600) {
            setError(
              "Account refresh is still running. Check its status before retrying.",
            );
            setPending(false);
            return;
          }
          timer.current = setTimeout(
            () =>
              void read({
                ...input,
                refresh: false,
                expectedIdentity: result.connectionIdentity,
                expectedSnapshotId: null,
              }),
            1000,
          );
        } else {
          setPending(false);
        }
      } catch (e) {
        if (token !== generation.current) return;
        setSnapshot(null);
        setError(
          e instanceof TypeError
            ? "The character snapshot is invalid."
            : toCommandError(e).messageKey,
        );
        setPending(false);
      }
    };
    void read(request);
  }, []);
  const cancel = useCallback(() => {
    ++generation.current;
    if (timer.current !== null) clearTimeout(timer.current);
  }, []);
  useEffect(() => {
    load(INITIAL_CHARACTER_REQUEST);
    return cancel;
  }, [load, cancel]);
  return { snapshot, pending, error, load };
}
