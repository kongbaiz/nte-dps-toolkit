import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { Check, Info, TriangleAlert, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cleanupAsyncRegistration } from "@/lib/async-cleanup";
import { t, tf, useTranslationRevision } from "@/lib/i18n";
import { MOTION_DURATION, waitForMotion } from "@/lib/motion";
import { islandClient, type IslandSnapshot } from "@/lib/tauri/island-client";
import { parseIslandCommandError } from "@/lib/tauri/island-contract";
import { cn } from "@/lib/utils";

export function IslandPage() {
  useTranslationRevision();
  const [snapshot, setSnapshot] = useState<IslandSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exiting, setExiting] = useState(false);
  const [undoPending, setUndoPending] = useState(false);
  const dismissing = useRef<string | null>(null);

  useEffect(() => {
    const refresh = () => {
      void islandClient
        .getSnapshot()
        .then(setSnapshot)
        .catch((value) => {
          const error = parseIslandCommandError(value);
          setError(tf(error.messageKey, error.messageArguments));
        });
    };
    refresh();
    return cleanupAsyncRegistration(
      islandClient.subscribe(refresh),
      (value) => {
        const error = parseIslandCommandError(value);
        setError(tf(error.messageKey, error.messageArguments));
      },
    );
  }, []);

  const dismissWithMotion = useCallback(async (noticeId: string) => {
    if (dismissing.current === noticeId) return;
    dismissing.current = noticeId;
    setExiting(true);
    await waitForMotion(MOTION_DURATION.slow);
    try {
      setSnapshot(await islandClient.dismiss(noticeId));
    } catch (value) {
      const commandError = parseIslandCommandError(value);
      setError(tf(commandError.messageKey, commandError.messageArguments));
      setExiting(false);
    } finally {
      dismissing.current = null;
    }
  }, []);

  useEffect(() => {
    setExiting(false);
    setUndoPending(false);
    setError(null);
  }, [snapshot?.notice?.id]);

  useEffect(() => {
    const notice = snapshot?.notice;
    if (notice === null || notice === undefined) return;
    const timer = window.setTimeout(
      () => void dismissWithMotion(notice.id),
      Math.max(250, notice.remainingMs),
    );
    return () => window.clearTimeout(timer);
  }, [dismissWithMotion, snapshot?.notice]);

  const notice = snapshot?.notice;
  if (notice === null || notice === undefined) return null;
  const Icon =
    notice.tone === "success"
      ? Check
      : notice.tone === "warning" || notice.tone === "error"
        ? TriangleAlert
        : Info;

  return (
    <main className="island-screen-layer grid h-screen w-screen place-items-start bg-transparent p-1.5">
      <section
        key={notice.id}
        className={cn(
          "motion-island pointer-events-auto flex h-full w-full items-center gap-3 rounded-full border border-white/10 bg-zinc-950/96 px-4 text-white shadow-2xl shadow-black/35 backdrop-blur-2xl",
          notice.tone === "error" && "text-red-300",
          notice.tone === "warning" && "text-amber-300",
          notice.tone === "success" && "text-emerald-300",
          notice.tone === "info" && "text-sky-300",
        )}
        data-tone={notice.tone}
        data-exiting={exiting}
        aria-live="polite"
        role="status"
        style={
          {
            "--island-duration": `${Math.max(250, notice.remainingMs)}ms`,
          } as CSSProperties
        }
      >
        <span className="motion-island-icon grid size-8 shrink-0 place-items-center rounded-full bg-white/8">
          <Icon className="size-5" aria-hidden="true" />
        </span>
        <span className="motion-island-content min-w-0 flex-1 text-white">
          <span className="block truncate text-sm font-medium">
            {tf(notice.messageKey, notice.messageArguments)}
          </span>
          {error !== null ? (
            <span className="block truncate text-[11px] text-red-300">
              {error}
            </span>
          ) : null}
        </span>
        {notice.undoAvailable ? (
          <Button
            size="sm"
            variant="outline"
            className="border-white/15 bg-white/8 text-white hover:bg-white/15 hover:text-white"
            disabled={undoPending || exiting}
            onClick={() => {
              setUndoPending(true);
              void islandClient
                .undo(notice.id)
                .then(setSnapshot)
                .catch((value) => {
                  const commandError = parseIslandCommandError(value);
                  setError(
                    tf(commandError.messageKey, commandError.messageArguments),
                  );
                  setUndoPending(false);
                });
            }}
          >
            {t(undoPending ? "Working..." : "Undo")}
          </Button>
        ) : null}
        <button
          aria-label={t("Close")}
          className="rounded-full p-1 text-white/65 transition-[color,background-color,transform] hover:bg-white/10 hover:text-white active:scale-90"
          disabled={exiting}
          onClick={() => void dismissWithMotion(notice.id)}
        >
          <X className="size-4" aria-hidden="true" />
        </button>
      </section>
    </main>
  );
}
