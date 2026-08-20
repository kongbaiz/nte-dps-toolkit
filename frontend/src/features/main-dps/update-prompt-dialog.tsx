import { Download, RefreshCw, Sparkles } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBackdrop,
  DialogClose,
  DialogDescription,
  DialogPopup,
  DialogPortal,
  DialogTitle,
} from "@/components/ui/dialog";
import { Markdown } from "@/components/ui/markdown";
import { t, tf } from "@/lib/i18n";
import type {
  AvailableUpdate,
  UpdateSettings,
} from "@/lib/tauri/settings-contract";
import {
  formatByteCount,
  formatUpdateByteProgress,
  updateComponentLabelKey,
  updateProgressPercent,
} from "@/lib/update-presentation";
import { cn } from "@/lib/utils";

interface UpdatePromptDialogProps {
  updates: UpdateSettings;
  pending: boolean;
  error: string | null;
  onLater(): void;
  onUpdate(): void;
}

export function UpdatePromptDialog({
  updates,
  pending,
  error,
  onLater,
  onUpdate,
}: UpdatePromptDialogProps) {
  const prepared = updates.prepared;
  const busy = ["checking", "downloading", "installing", "restarting"].includes(
    updates.status,
  );
  const canUpdate =
    !pending &&
    !busy &&
    (prepared === null || updates.installEnabled) &&
    updates.available.length > 0;
  const primary = preferredUpdate(updates.available);
  const progress = updateProgressPercent(
    updates.downloadedBytes,
    updates.totalBytes,
  );

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onLater();
      }}
    >
      <DialogPortal>
        <DialogBackdrop className="z-[90]" />
        <DialogPopup className="ui-motion-dialog z-[91] top-1/2 left-1/2 flex max-h-[min(42rem,calc(100vh-2rem))] w-[calc(100vw-2rem)] max-w-2xl -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-2xl border bg-background shadow-2xl">
          <header className="flex items-start gap-3 border-b p-5">
            <span className="grid size-10 shrink-0 place-items-center rounded-xl bg-primary/10 text-primary">
              <Sparkles className="size-5" aria-hidden="true" />
            </span>
            <div className="min-w-0 flex-1">
              <DialogTitle id="startup-update-title">
                {t("Update available")}
              </DialogTitle>
              <DialogDescription className="mt-1">
                {primary === null
                  ? t("A new version of NTE DPS TOOL is available.")
                  : tf("NTE DPS TOOL {} is available.", [primary.version])}
              </DialogDescription>
            </div>
          </header>

          <div className="min-h-0 space-y-4 overflow-y-auto p-5">
            <div>
              <h3 className="text-sm font-medium">{t("Release notes")}</h3>
              <div className="mt-2 space-y-3">
                {updates.available.map((update) => (
                  <ReleaseNotes key={update.component} update={update} />
                ))}
              </div>
            </div>

            {updates.status !== "available" && updates.status !== "ready" ? (
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                {busy ? (
                  <RefreshCw
                    className="size-3.5 animate-spin"
                    aria-hidden="true"
                  />
                ) : null}
                <span>{tf(updates.messageKey, updates.messageArguments)}</span>
                {updates.status === "downloading" ? (
                  <span>
                    {formatUpdateByteProgress(
                      updates.downloadedBytes,
                      updates.totalBytes,
                    )}
                    {progress === null ? null : ` (${progress}%)`}
                  </span>
                ) : null}
              </div>
            ) : null}
            {updates.installBlockedMessageKey ? (
              <p className="text-xs text-destructive">
                {t(updates.installBlockedMessageKey)}
              </p>
            ) : null}
            {error !== null ? (
              <p className="text-xs text-destructive" role="alert">
                {error}
              </p>
            ) : null}
          </div>

          <footer className="flex flex-wrap justify-end gap-2 border-t p-4">
            <DialogClose render={<Button type="button" variant="outline" />}>
              {t("Later")}
            </DialogClose>
            <Button type="button" disabled={!canUpdate} onClick={onUpdate}>
              <Download aria-hidden="true" />
              {pending
                ? t("Updating...")
                : prepared?.component === "app"
                  ? t("Install and restart")
                  : prepared?.component === "mods-plugin"
                    ? t("Install Mod loader update")
                    : t("Update now")}
            </Button>
          </footer>
        </DialogPopup>
      </DialogPortal>
    </Dialog>
  );
}

function ReleaseNotes({ update }: { update: AvailableUpdate }) {
  return (
    <article className="rounded-xl border bg-muted/20 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="outline">
          {t(updateComponentLabelKey(update.component))}
        </Badge>
        <strong className="text-sm">{update.version}</strong>
        <span className="text-xs text-muted-foreground">
          {update.publishedAt} · {formatByteCount(update.artifactSize)}
        </span>
      </div>
      <Markdown
        className={cn(
          "mt-2 text-xs leading-relaxed text-muted-foreground",
          !update.notes.trim() && "italic",
        )}
      >
        {update.notes.trim() || t("No release notes are available.")}
      </Markdown>
    </article>
  );
}

function preferredUpdate(
  updates: readonly AvailableUpdate[],
): AvailableUpdate | null {
  return (
    updates.find((update) => update.component === "app") ?? updates[0] ?? null
  );
}
