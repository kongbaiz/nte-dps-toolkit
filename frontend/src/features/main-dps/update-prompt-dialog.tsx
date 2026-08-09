import { Download, RefreshCw, Sparkles } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { t, tf } from "@/lib/i18n";
import type {
  AvailableUpdate,
  UpdateComponentId,
  UpdateSettings,
} from "@/lib/tauri/settings-contract";
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
    <div className="ui-motion-overlay fixed inset-0 z-[90] grid place-items-center bg-black/55 p-4">
      <section
        aria-labelledby="startup-update-title"
        aria-modal="true"
        className="ui-motion-dialog flex max-h-[min(42rem,calc(100vh-2rem))] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border bg-background shadow-2xl"
        role="dialog"
      >
        <header className="flex items-start gap-3 border-b p-5">
          <span className="grid size-10 shrink-0 place-items-center rounded-xl bg-primary/10 text-primary">
            <Sparkles className="size-5" aria-hidden="true" />
          </span>
          <div className="min-w-0 flex-1">
            <h2 id="startup-update-title" className="text-lg font-semibold">
              {t("Update available")}
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">
              {primary === null
                ? t("A new version of NTE DPS TOOL is available.")
                : tf("NTE DPS TOOL {} is available.", [primary.version])}
            </p>
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
                  {formatByteCount(updates.downloadedBytes)} /{" "}
                  {formatByteCount(updates.totalBytes)} ({progress}%)
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
          <Button type="button" variant="outline" onClick={onLater}>
            {t("Later")}
          </Button>
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
      </section>
    </div>
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
      <div
        className={cn(
          "mt-2 whitespace-pre-wrap text-xs leading-relaxed text-muted-foreground",
          !update.notes.trim() && "italic",
        )}
      >
        {update.notes.trim() || t("No release notes are available.")}
      </div>
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

function updateComponentLabelKey(component: UpdateComponentId): string {
  return component === "app" ? "Application" : "Mod loader";
}

function updateProgressPercent(downloaded: string, total: string): number {
  const totalBytes = BigInt(total);
  if (totalBytes === 0n) return 0;
  return Number((BigInt(downloaded) * 100n) / totalBytes);
}

function formatByteCount(value: string): string {
  const bytes = BigInt(value);
  const units = [
    { size: 1024n * 1024n * 1024n, suffix: "GB" },
    { size: 1024n * 1024n, suffix: "MB" },
    { size: 1024n, suffix: "KB" },
  ];
  const unit = units.find((candidate) => bytes >= candidate.size);
  if (!unit) return `${bytes} B`;
  const tenths = (bytes * 10n) / unit.size;
  return `${tenths / 10n}.${tenths % 10n} ${unit.suffix}`;
}
