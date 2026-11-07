import { lazy, Suspense } from "react";
import { Power, Puzzle } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ActionNotice } from "@/components/nte/action-notice";
import { usePluginFormCatalog } from "./plugin-form-catalog";
import { t } from "@/lib/i18n";
import type { PluginPanel } from "@/lib/tauri/generated/PluginPanel";
import { toolkitClient } from "@/lib/tauri/toolkit-client";
import { PLUGIN_STATE } from "./release-control-groups";
const PluginFormPanel = lazy(() =>
  import("./plugin-form-panel").then((module) => ({
    default: module.PluginFormPanel,
  })),
);

export function PluginControls({
  panel,
  plugin,
  title,
  pending,
  run,
}: {
  panel: PluginPanel;
  plugin: PluginPanel["plugins"][number];
  title: string;
  pending: boolean;
  run: (action: () => Promise<unknown>) => Promise<void>;
}) {
  const entry = usePluginFormCatalog((state) => state.entries[plugin.file]);
  const reload = usePluginFormCatalog((state) => state.load);
  const loaded = plugin.state === "loaded";
  const resident = loaded || plugin.state === "faulted";
  const busy =
    pending ||
    panel.collectorActive ||
    panel.combat?.capturing === true ||
    ["unload_pending", "blocked", "release_pending", "validated"].includes(
      plugin.state,
    ) ||
    panel.operation?.state === "queued" ||
    panel.operation?.state === "pending" ||
    panel.connection !== "connected" ||
    panel.mode !== "plugin";
  return (
    <div
      className="flex min-w-0 flex-col gap-4"
      data-testid="plugin-control-page"
    >
      <header
        className="flex flex-wrap items-center justify-between gap-3 border-b pb-3"
        data-testid="plugin-page-header"
      >
        <div className="flex min-w-0 items-center gap-3">
          <Puzzle className="size-5 shrink-0" />
          <h2 className="truncate text-xl font-semibold">{title}</h2>
          <Badge variant="outline">{t(PLUGIN_STATE[plugin.state])}</Badge>
        </div>
        <Button
          size="sm"
          variant={resident ? "outline" : "default"}
          disabled={busy || !panel.capabilities.includes(resident ? 5 : 4)}
          onClick={() =>
            void run(() =>
              toolkitClient.control(
                resident ? "disable" : "enable",
                plugin.file,
                panel.connectionIdentity!,
              ),
            )
          }
        >
          <Power />
          {t(resident ? "Unload plugin" : "Load plugin")}
        </Button>
      </header>
      {(panel.collectorActive || panel.combat?.capturing) && (
        <p className="text-sm text-muted-foreground">
          {t("Stop collection before changing plugin lifecycle state.")}
        </p>
      )}
      {loaded &&
      panel.connection === "connected" &&
      panel.mode === "plugin" &&
      panel.connectionIdentity ? (
        entry?.error ? (
          <div className="space-y-3">
            <ActionNotice status="error" message={t(entry.error.messageKey)} />
            <Button
              variant="outline"
              disabled={pending}
              onClick={() =>
                void reload(
                  plugin.file,
                  panel.connectionIdentity!,
                  plugin.serviceOwner!,
                  true,
                )
              }
            >
              {t("Retry")}
            </Button>
          </div>
        ) : entry?.snapshot &&
          entry.key ===
            JSON.stringify([panel.connectionIdentity, plugin.serviceOwner]) &&
          !entry.loading &&
          entry.snapshot.owner === plugin.serviceOwner ? (
          <Suspense
            fallback={
              <p className="text-sm text-muted-foreground">{t("Loading…")}</p>
            }
          >
            <PluginFormPanel
              key={plugin.file + ":" + entry.snapshot.owner}
              snapshot={entry.snapshot}
              file={plugin.file}
              identity={panel.connectionIdentity}
              pending={pending}
              capturing={
                panel.collectorActive || panel.combat?.capturing === true
              }
              run={run}
            />
          </Suspense>
        ) : (
          <p className="text-sm text-muted-foreground">{t("Loading…")}</p>
        )
      ) : (
        <p className="text-sm text-muted-foreground">
          {t("Load the plugin to view its controls.")}
        </p>
      )}
    </div>
  );
}
