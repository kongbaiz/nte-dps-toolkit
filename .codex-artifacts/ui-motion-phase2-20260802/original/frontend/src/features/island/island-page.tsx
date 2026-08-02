import { useEffect, useState } from "react";
import { Check, Info, TriangleAlert, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { t, tf, useTranslationRevision } from "@/lib/i18n";
import { islandClient, type IslandSnapshot } from "@/lib/tauri/island-client";
import { parseIslandCommandError } from "@/lib/tauri/island-contract";
import { cn } from "@/lib/utils";

export function IslandPage() {
  useTranslationRevision();
  const [snapshot, setSnapshot] = useState<IslandSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

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

  useEffect(() => {
    const notice = snapshot?.notice;
    if (notice === null || notice === undefined) return;
    const timer = window.setTimeout(
      () => void islandClient.dismiss(notice.id).then(setSnapshot),
      Math.max(250, notice.remainingMs),
    );
    return () => window.clearTimeout(timer);
  }, [snapshot?.notice]);

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
        className={cn(
          "flex h-full items-center gap-3 rounded-2xl border bg-background/95 px-4 shadow-2xl backdrop-blur-xl",
          notice.tone === "error" && "border-destructive/40",
          notice.tone === "warning" && "border-amber-500/40",
          notice.tone === "success" && "border-emerald-500/40",
        )}
      >
        <Icon className="size-5 shrink-0" aria-hidden="true" />
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
            onClick={() =>
              void islandClient
                .undo(notice.id)
                .then(setSnapshot)
                .catch((value) => {
                  const error = parseIslandCommandError(value);
                  setError(tf(error.messageKey, error.messageArguments));
                })
            }
          >
            {t("Undo")}
          </Button>
        ) : null}
        <button
          aria-label={t("Close")}
          className="rounded-md p-1 hover:bg-muted"
          onClick={() => void islandClient.dismiss(notice.id).then(setSnapshot)}
        >
          <X className="size-4" />
        </button>
      </section>
    </main>
  );
}
