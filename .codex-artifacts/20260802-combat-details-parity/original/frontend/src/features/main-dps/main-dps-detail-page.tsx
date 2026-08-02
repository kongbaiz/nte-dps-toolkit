import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { DesktopTitlebar } from "@/components/nte/desktop-titlebar";
import { Button } from "@/components/ui/button";
import { t, tf, useTranslationRevision } from "@/lib/i18n";
import { useSettingsPresentation } from "@/lib/settings-presentation";
import { mainDpsDetailClient } from "@/lib/tauri/main-dps-detail-client";
import type { MainDpsDetailSnapshot } from "@/lib/tauri/main-dps-detail-contract";

import { formatMainMetric } from "./main-dps-model";

export function MainDpsDetailPage() {
  useTranslationRevision();
  useSettingsPresentation();
  const [snapshot, setSnapshot] = useState<MainDpsDetailSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      setSnapshot(await mainDpsDetailClient.getSnapshot());
    } catch (loadError) {
      setError(
        loadError instanceof Error ? loadError.message : String(loadError),
      );
    } finally {
      setPending(false);
    }
  }, []);

  const loadMore = useCallback(async () => {
    if (snapshot === null || pending) return;
    setPending(true);
    setError(null);
    try {
      const next = await mainDpsDetailClient.getSnapshot(snapshot.rows.length);
      setSnapshot((current) =>
        current !== null &&
        current.kind === next.kind &&
        current.characterId === next.characterId &&
        current.filter === next.filter
          ? { ...next, offset: 0, rows: [...current.rows, ...next.rows] }
          : next,
      );
    } catch (loadError) {
      setError(
        loadError instanceof Error ? loadError.message : String(loadError),
      );
    } finally {
      setPending(false);
    }
  }, [pending, snapshot]);

  useEffect(() => {
    void load();
    let unlisten: (() => void) | null = null;
    void mainDpsDetailClient
      .subscribeRequested(() => void load())
      .then((value) => {
        unlisten = value;
      });
    return () => unlisten?.();
  }, [load]);

  const title =
    snapshot?.kind === "character"
      ? tf("{} Combat Details", [snapshot.characterName ?? t("Character")])
      : t("Team Combat Details");

  return (
    <div className="desktop-window-shell">
      <DesktopTitlebar title={title} />
      <main className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden p-4">
        <div className="flex items-center justify-between gap-3 border-b pb-3">
          <div className="min-w-0">
            <h1 className="truncate text-lg font-semibold">{title}</h1>
            <p className="text-sm text-muted-foreground">
              {snapshot === null
                ? t("Loading")
                : tf("{} hits · {} damage", [
                    String(snapshot.totalHits),
                    formatMainMetric(snapshot.totalDamage),
                  ])}
            </p>
          </div>
          <Button size="sm" variant="outline" disabled={pending} onClick={load}>
            <RefreshCw className={pending ? "animate-spin" : ""} />
            {t("Refresh")}
          </Button>
        </div>

        {error !== null ? (
          <div className="m-auto rounded-lg border border-destructive/30 px-4 py-3 text-sm text-destructive">
            {error}
          </div>
        ) : snapshot === null ? (
          <div className="m-auto size-7 animate-spin rounded-full border-2 border-muted border-t-foreground" />
        ) : snapshot.rows.length === 0 ? (
          <div className="m-auto text-sm text-muted-foreground">
            {t("No matching combat hits")}
          </div>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col gap-2">
            <div className="min-h-0 flex-1 overflow-auto rounded-lg border bg-card">
              <table className="w-full table-fixed text-sm">
                <thead className="sticky top-0 z-10 bg-muted text-left text-xs text-muted-foreground">
                  <tr>
                    <th className="w-28 px-3 py-2 font-medium">{t("Time")}</th>
                    <th className="w-36 px-3 py-2 font-medium">
                      {t("Character")}
                    </th>
                    <th className="px-3 py-2 font-medium">{t("Skill")}</th>
                    <th className="px-3 py-2 font-medium">{t("Target")}</th>
                    <th className="w-32 px-3 py-2 text-right font-medium">
                      {t("Damage")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {snapshot.rows.map((row) => (
                    <tr key={row.id} className="border-t hover:bg-muted/50">
                      <td className="truncate px-3 py-2 font-mono text-xs text-muted-foreground">
                        {row.timestamp.toFixed(3)}
                      </td>
                      <td className="truncate px-3 py-2">
                        {row.characterName}
                      </td>
                      <td className="truncate px-3 py-2" title={row.skill}>
                        {row.skill}
                      </td>
                      <td className="truncate px-3 py-2" title={row.target}>
                        {row.target}
                      </td>
                      <td className="px-3 py-2 text-right font-mono tabular-nums">
                        {row.direction === "incoming" ? "−" : ""}
                        {formatMainMetric(row.damage)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <div className="flex items-center justify-between text-xs text-muted-foreground">
              <span>
                {tf("Showing {} of {} hits", [
                  String(snapshot.rows.length),
                  String(snapshot.totalHits),
                ])}
              </span>
              {snapshot.rows.length < snapshot.totalHits ? (
                <Button
                  size="sm"
                  variant="outline"
                  disabled={pending}
                  onClick={loadMore}
                >
                  {t("Load more")}
                </Button>
              ) : null}
            </div>
          </div>
        )}
      </main>
    </div>
  );
}
