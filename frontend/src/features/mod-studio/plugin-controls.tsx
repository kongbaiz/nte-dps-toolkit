import {
  Power,
  RefreshCw,
  Puzzle,
  Activity,
  Users,
  Target,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { t } from "@/lib/i18n";
import { toolkitClient, type PluginPanel } from "@/lib/tauri/toolkit-client";
import { GameHudControls } from "./game-hud-controls";
import { ReleaseControls } from "./release-controls";
import { pluginControlGroup } from "./release-control-groups";
const PLUGIN_STATE = {
  loaded: "Loaded",
  unloaded: "Disabled",
  unload_pending: "Unloading",
  failed: "Load failed",
} as const;
export function PluginControls({
  panel,
  plugin,
  title,
  pending,
  run,
  refresh,
}: {
  panel: PluginPanel;
  plugin: PluginPanel["plugins"][number];
  title: string;
  pending: boolean;
  refresh: () => Promise<void>;
  run: (action: () => Promise<unknown>) => Promise<void>;
}) {
  const group = pluginControlGroup(plugin.file);
  const Icon =
    group?.id === "combat"
      ? Activity
      : group?.id === "account"
        ? Users
        : group?.id === "training"
          ? Target
          : Puzzle;
  const loaded = plugin.state === "loaded";
  const busy =
    pending ||
    panel.collectorActive ||
    panel.combat?.capturing === true ||
    plugin.state === "unload_pending" ||
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
        <div className="flex min-w-0 items-center gap-4">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-xl border border-primary/10 bg-primary/10 text-primary">
            <Icon className="size-6" />
          </div>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2.5">
              <h2 className="text-lg font-semibold tracking-tight">{title}</h2>
              <Badge variant="outline">{t(PLUGIN_STATE[plugin.state])}</Badge>
            </div>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            variant={loaded ? "outline" : "default"}
            disabled={busy || !panel.capabilities.includes(loaded ? 5 : 4)}
            onClick={() =>
              void run(() =>
                toolkitClient.control(
                  loaded ? "disable" : "enable",
                  plugin.file,
                ),
              )
            }
          >
            <Power />
            {t(loaded ? "Unload plugin" : "Load plugin")}
          </Button>
        </div>
        {panel.collectorActive && (
          <p className="w-full text-xs text-muted-foreground">
            {t("Stop collection before changing plugin lifecycle state.")}
          </p>
        )}
        {panel.operation?.state && (
          <p className="w-full text-xs text-muted-foreground">
            {t("Plugin operation")}: {panel.operation.state}
            {panel.operation.error && ` · ${t(panel.operation.error)}`}
          </p>
        )}
      </header>
      <div
        className={`grid min-w-0 items-start gap-4 ${group?.id === "combat" ? "@min-[48rem]/detail:grid-cols-2" : "w-full"}`}
        data-testid="plugin-settings-grid"
      >
        {group?.id === "combat" && (
          <GameHudControls panel={panel} pending={pending} run={run} />
        )}
        {group ? (
          <ReleaseControls
            panel={panel}
            pending={pending}
            run={run}
            group={group}
          />
        ) : (
          <Card>
            <CardContent className="flex gap-3 text-sm text-muted-foreground">
              <Puzzle className="size-5 shrink-0" />
              <p>
                {t(
                  "This plugin has no feature controls exposed to Toolkit. Its lifecycle controls are available on this page.",
                )}
              </p>
            </CardContent>
          </Card>
        )}
      </div>
      <details className="rounded-lg border px-4 py-3 text-xs text-muted-foreground">
        <summary className="cursor-pointer font-medium focus-visible:outline-ring">
          {t("Plugin details")}
        </summary>
        <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
          <p className="break-all font-mono">{plugin.file}</p>
          <Button
            size="sm"
            variant="outline"
            disabled={pending}
            onClick={() => void refresh()}
          >
            <RefreshCw />
            {t("Check status")}
          </Button>
        </div>
      </details>
    </div>
  );
}
