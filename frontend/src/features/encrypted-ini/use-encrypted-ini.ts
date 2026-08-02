import { useCallback, useEffect, useRef, useState } from "react";

import { encryptedIniClient } from "@/lib/tauri/encrypted-ini-client";
import {
  encryptedIniError,
  type EncryptedIniKey,
  type EncryptedIniSnapshot,
} from "@/lib/tauri/encrypted-ini-contract";

export interface EncryptedIniNotice {
  kind: "success" | "error";
  titleKey: string;
  messageKey: string;
  messageArguments: string[];
}

export function useEncryptedIni() {
  const mounted = useRef(true);
  const [snapshot, setSnapshot] = useState<EncryptedIniSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<EncryptedIniNotice | null>(null);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const loadSnapshot = useCallback(async () => {
    setLoading(true);
    try {
      const next = await encryptedIniClient.getSnapshot();
      if (mounted.current) {
        setSnapshot((current) =>
          current?.generation === next.generation ? current : next,
        );
      }
    } catch (error) {
      if (mounted.current) setNotice(errorNotice(error));
    } finally {
      if (mounted.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadSnapshot();
  }, [loadSnapshot]);

  const open = useCallback(async () => {
    setBusy(true);
    setNotice(null);
    try {
      const result = await encryptedIniClient.open();
      if (!mounted.current) return false;
      if (result.opened) {
        setSnapshot(result.snapshot);
        setNotice({
          kind: "success",
          titleKey: "Encrypted INI opened",
          messageKey: "Parsed {} lines of ciphertext using the {} key",
          messageArguments: [
            String(result.snapshot.encryptedLineCount),
            result.snapshot.key,
          ],
        });
      }
      return result.opened;
    } catch (error) {
      if (mounted.current) setNotice(errorNotice(error));
      return false;
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, []);

  const reload = useCallback(async () => {
    setBusy(true);
    setNotice(null);
    try {
      const next = await encryptedIniClient.reload();
      if (!mounted.current) return false;
      setSnapshot(next);
      setNotice({
        kind: "success",
        titleKey: "Encrypted INI reloaded",
        messageKey: "Parsed {} lines of ciphertext using the {} key",
        messageArguments: [String(next.encryptedLineCount), next.key],
      });
      return true;
    } catch (error) {
      if (mounted.current) setNotice(errorNotice(error));
      return false;
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, []);

  const save = useCallback(
    async (plaintext: string, key: EncryptedIniKey) => {
      if (!snapshot) return false;
      setBusy(true);
      setNotice(null);
      try {
        const result = await encryptedIniClient.save({
          expectedGeneration: snapshot.generation,
          plaintext,
          key,
        });
        if (!mounted.current) return false;
        setSnapshot(result.snapshot);
        setNotice({
          kind: "success",
          titleKey: result.saved
            ? "Encrypted INI saved"
            : "Encrypted INI unchanged",
          messageKey: result.saved
            ? "Saved encrypted INI using the {} key"
            : "Content unchanged; the original ciphertext file was kept",
          messageArguments: result.saved ? [result.snapshot.key] : [],
        });
        return true;
      } catch (error) {
        if (mounted.current) setNotice(errorNotice(error));
        return false;
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [snapshot],
  );

  const clear = useCallback(async () => {
    setBusy(true);
    setNotice(null);
    try {
      const next = await encryptedIniClient.clear();
      if (!mounted.current) return false;
      setSnapshot(next);
      setNotice({
        kind: "success",
        titleKey: "Encrypted INI",
        messageKey: "Encrypted INI editor cleared",
        messageArguments: [],
      });
      return true;
    } catch (error) {
      if (mounted.current) setNotice(errorNotice(error));
      return false;
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, []);

  return {
    snapshot,
    loading,
    busy,
    notice,
    open,
    reload,
    save,
    clear,
    clearNotice: () => setNotice(null),
  };
}

function errorNotice(error: unknown): EncryptedIniNotice {
  const parsed = encryptedIniError(error);
  return {
    kind: "error",
    titleKey: "Encrypted INI operation failed",
    messageKey: parsed.messageKey,
    messageArguments: parsed.messageArguments,
  };
}
