import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { Check, Info, TriangleAlert, X } from "lucide-react";

import { Button } from "@/components/ui/button";
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
    let unlisten: (() => void) | undefined;
    void islandClient.subscribe(refresh).then((next) => {
      unlisten = next;
    });
    return () => unlisten?.();
  }, []);

  const dismissWithMotion = useCallback(async (noticeId: string) => {
    if (dismissing.current === noticeId) return;
    dismissing.current = noticeId;
    setExiting(true);
    await waitForMotion(MOTION_DURATION.base);
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
    <main className="h-screen w-screen bg-transparent p-1.5">
      <section
        key={notice.id}
        className={cn(
          "motion-island flex h-full items-center gap-3 rounded-2xl border bg-background/95 px-4 shadow-2xl backdrop-blur-xl",
          notice.tone === "error" && "border-destructive/40",
          notice.tone === "warning" && "border-amber-500/40",
          notice.tone === "success" && "border-emerald-500/40",
        )}
        data-exiting={exiting}
        style={
          {
            "--island-duration": `${Math.max(250, notice.remainingMs)}ms`,
          } as CSSProperties
        }
      >
        <span className="motion-island-icon grid shrink-0 place-items-center">
          <Icon className="size-5" aria-hidden="true" />
        </span>
        <span className="min-w-0 flex-1 truncate text-sm">
          {tf(notice.messageKey, notice.messageArguments)}
        </span>
        {error !== null ? (
          <span className="text-xs text-destructive">{error}</span>
        ) : null}
        {notice.undoAvailable ? (
          <Button
            size="sm"
            variant="outline"
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
          className="rounded-md p-1 transition-[background-color,transform] hover:bg-muted active:scale-90"
          disabled={exiting}
          onClick={() => void dismissWithMotion(notice.id)}
        >
          <X className="size-4" aria-hidden="true" />
        </button>
      </section>
    </main>
  );
}
