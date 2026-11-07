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
import { t, tf, useFrontendLanguage } from "@/lib/i18n";
import { MOTION_DURATION, waitForMotion } from "@/lib/motion";
import { toCommandError } from "@/lib/tauri/command-error";
import type { IslandSnapshot } from "@/lib/tauri/generated/IslandSnapshot";
import { islandClient } from "@/lib/tauri/island-client";
import { cn } from "@/lib/utils";

export function IslandPage() {
  useFrontendLanguage();
  const [snapshot, setSnapshot] = useState<IslandSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exiting, setExiting] = useState(false);
  const [undoPending, setUndoPending] = useState(false);
  const dismissing = useRef<string | null>(null);

  const lifecycle = useRef({ mounted: false, serial: 0 });
  const currentNotice = useRef<string | null>(null);
  const acceptSnapshot = useCallback((next: IslandSnapshot, ticket: number) => {
    if (!lifecycle.current.mounted || ticket !== lifecycle.current.serial)
      return;
    currentNotice.current = next.notice?.id ?? null;
    setSnapshot(next);
    setError(null);
  }, []);
  useEffect(() => {
    const lifetime = lifecycle.current;
    lifetime.mounted = true;
    let active = true;
    const refresh = () => {
      if (!active) return;
      const ticket = ++lifecycle.current.serial;
      void islandClient
        .getSnapshot()
        .then((next) => acceptSnapshot(next, ticket))
        .catch((value) => {
          if (!lifecycle.current.mounted || ticket !== lifecycle.current.serial)
            return;
          const error = toCommandError(value);
          setError(tf(error.messageKey, error.messageArguments));
        });
    };
    refresh();
    const cleanup = cleanupAsyncRegistration(
      islandClient.subscribe(refresh),
      (value) => {
        if (!active) return;
        const error = toCommandError(value);
        setError(tf(error.messageKey, error.messageArguments));
      },
    );
    return () => {
      active = false;
      lifetime.mounted = false;
      lifetime.serial++;
      cleanup();
    };
  }, [acceptSnapshot]);

  const dismissWithMotion = useCallback(
    async (noticeId: string) => {
      if (dismissing.current === noticeId || currentNotice.current !== noticeId)
        return;
      dismissing.current = noticeId;
      setExiting(true);
      let ticket: number | undefined;
      try {
        await waitForMotion(MOTION_DURATION.slow);
        if (!lifecycle.current.mounted || currentNotice.current !== noticeId)
          return;
        ticket = ++lifecycle.current.serial;
        acceptSnapshot(await islandClient.dismiss(noticeId), ticket);
      } catch (value) {
        if (!lifecycle.current.mounted || ticket !== lifecycle.current.serial)
          return;
        const commandError = toCommandError(value);
        setError(tf(commandError.messageKey, commandError.messageArguments));
        setExiting(false);
      } finally {
        if (dismissing.current === noticeId) dismissing.current = null;
      }
    },
    [acceptSnapshot],
  );

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
      : notice.tone === "error"
        ? TriangleAlert
        : Info;

  return (
    <main className="island-screen-layer grid h-screen w-screen place-items-start bg-transparent p-1.5">
      <section
        key={notice.id}
        className={cn(
          "motion-island pointer-events-auto flex h-full w-full items-center gap-3 rounded-full border border-foreground/10 bg-background/96 px-4 text-foreground shadow-2xl shadow-black/35 backdrop-blur-2xl",
          notice.tone === "error" && "text-destructive",
          notice.tone === "success" && "text-success",
          notice.tone === "info" && "text-info",
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
        <span className="motion-island-icon grid size-8 shrink-0 place-items-center rounded-full bg-foreground/8">
          <Icon className="size-5" aria-hidden="true" />
        </span>
        <span className="motion-island-content min-w-0 flex-1 text-foreground">
          <span className="block truncate text-sm font-medium">
            {tf(notice.messageKey, notice.messageArguments)}
          </span>
          {error !== null ? (
            <span className="block truncate text-xs text-destructive">
              {error}
            </span>
          ) : null}
        </span>
        {notice.undoAvailable ? (
          <Button
            size="sm"
            variant="outline"
            className="border-foreground/15 bg-foreground/8 text-foreground hover:bg-foreground/15 hover:text-foreground"
            disabled={undoPending || exiting}
            onClick={() => {
              setUndoPending(true);
              const ticket = ++lifecycle.current.serial;
              void islandClient
                .undo(notice.id)
                .then((next) => acceptSnapshot(next, ticket))
                .catch((value) => {
                  if (
                    !lifecycle.current.mounted ||
                    ticket !== lifecycle.current.serial
                  )
                    return;
                  const commandError = toCommandError(value);
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
          className="rounded-full p-1 text-foreground/65 transition-[color,background-color,transform] hover:bg-foreground/10 hover:text-foreground active:scale-90"
          disabled={exiting}
          onClick={() => void dismissWithMotion(notice.id)}
        >
          <X className="size-4" aria-hidden="true" />
        </button>
      </section>
    </main>
  );
}
