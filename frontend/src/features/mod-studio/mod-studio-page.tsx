import { useCallback, useEffect, useRef, useState } from "react";
import {
  Activity,
  Check,
  Cpu,
  Network,
  Server,
  Users,
  Settings2,
  Store,
  TriangleAlert,
} from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { t, useTranslationRevision } from "@/lib/i18n";
import { toolkitClient, type PluginPanel } from "@/lib/tauri/toolkit-client";
import { HostControls } from "./host-controls";
import { PluginControls } from "./plugin-controls";
import { pluginControlGroup } from "./release-control-groups";
import { ModMarketPanel } from "./mod-market-panel";
const PLUGIN_STATE = {
  loaded: "Loaded",
  unloaded: "Disabled",
  unload_pending: "Unloading",
  failed: "Load failed",
} as const;
const PLUGIN_NAMES: Record<string, string> = {
  "nte_plugincombat.dll": "Combat plugin",
  "nte_pluginuser.dll": "Account plugin",
  "nte_pluginnetwork.dll": "Network plugin",
  "nte_pluginperformance.dll": "Performance plugin",
};
const ICONS = {
  combat: Activity,
  account: Users,
  network: Network,
  host: Server,
};
export function ModStudioWorkspace() {
  useTranslationRevision();
  const [panel, setPanel] = useState<PluginPanel | null>(null);
  const [tab, setTab] = useState<"control" | "market">("control");
  const [selectedPage, setSelectedPage] = useState("host");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const serial = useRef(0);
  const busy = useRef(false);
  const refresh = useCallback(async () => {
    const request = ++serial.current;
    try {
      const snapshot = await toolkitClient.snapshot();
      if (alive.current && request === serial.current) setPanel(snapshot);
    } catch {
      if (alive.current && request === serial.current) {
        setPanel(null);
        setError("Failed to read the plugin control panel.");
      }
    }
  }, []);
  useEffect(() => {
    alive.current = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      if (!busy.current && document.visibilityState === "visible")
        await refresh();
      if (alive.current) timer = setTimeout(() => void poll(), 3000);
    };
    void poll();
    return () => {
      alive.current = false;
      serial.current += 1;
      clearTimeout(timer);
    };
  }, [refresh]);
  const run = async (action: () => Promise<unknown>) => {
    if (busy.current) return;
    busy.current = true;
    serial.current += 1;
    setPending(true);
    setError(null);
    try {
      await action();
      if (alive.current) await refresh();
    } catch (e) {
      if (alive.current)
        setError(
          typeof e === "object" &&
            e !== null &&
            "messageKey" in e &&
            typeof e.messageKey === "string"
            ? e.messageKey
            : "The plugin operation failed. Query its status before retrying.",
        );
    } finally {
      busy.current = false;
      if (alive.current) setPending(false);
    }
  };
  const selectedPlugin = panel?.plugins.find(
    (plugin) => plugin.file === selectedPage,
  );
  const activePage = selectedPlugin?.file ?? "host";
  const selectPage = (page: string) => {
    setSelectedPage(page);
    setError(null);
    contentRef.current?.scrollTo({ top: 0 });
  };
  const errorBanner = error ? (
    <Alert variant="destructive" className="shrink-0">
      <TriangleAlert />
      <AlertTitle>{t("Operation failed")}</AlertTitle>
      <AlertDescription>{t(error)}</AlertDescription>
    </Alert>
  ) : null;
  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <header className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b px-5 py-4">
        <div>
          <h1 className="text-lg font-semibold">{t("Mod Workshop")}</h1>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t(
              "Install components, connect the host, then manage your plugins.",
            )}
          </p>
        </div>
        <div
          className="flex items-center gap-1 rounded-lg bg-muted/70 p-1"
          aria-label={t("Mod Workshop")}
        >
          <Button
            size="sm"
            variant={tab === "control" ? "default" : "ghost"}
            aria-pressed={tab === "control"}
            disabled={pending}
            onClick={() => setTab("control")}
          >
            {tab === "control" ? <Check aria-hidden="true" /> : <Settings2 />}
            {t("Control panel")}
          </Button>
          <Button
            size="sm"
            variant={tab === "market" ? "default" : "ghost"}
            aria-pressed={tab === "market"}
            disabled={pending}
            onClick={() => setTab("market")}
          >
            {tab === "market" ? <Check aria-hidden="true" /> : <Store />}
            {t("Mod Market")}
          </Button>
        </div>
      </header>
      {tab === "market" ? (
        <div className="min-h-0 flex-1 overflow-y-auto" ref={contentRef}>
          <div className="mx-auto w-full max-w-7xl space-y-5 p-5">
            {errorBanner}
            <ModMarketPanel onInstalled={refresh} />
          </div>
        </div>
      ) : panel === null ? (
        <div className="space-y-4 p-5">
          {errorBanner}
          <Skeleton className="h-40" />
        </div>
      ) : (
        <div
          className="flex min-h-0 flex-1 flex-col"
          data-testid="plugin-split-view"
        >
          <nav
            aria-label={t("Plugin control pages")}
            className="flex shrink-0 items-center gap-2 overflow-x-auto border-b bg-muted/25 px-4 py-2"
          >
            <Button
              variant={activePage === "host" ? "secondary" : "ghost"}
              className="shrink-0 justify-start"
              aria-current={activePage === "host" ? "page" : undefined}
              disabled={pending}
              onClick={() => selectPage("host")}
            >
              <Server />
              {t("Host controls")}
            </Button>
            <div className="flex shrink-0 items-center gap-1 border-l pl-2">
              {panel.plugins.map((plugin) => {
                const group = pluginControlGroup(plugin.file);
                const Icon = group ? ICONS[group.id] : Cpu;
                return (
                  <button
                    type="button"
                    key={plugin.file}
                    disabled={pending}
                    aria-current={
                      activePage === plugin.file ? "page" : undefined
                    }
                    onClick={() => selectPage(plugin.file)}
                    className={`flex min-w-0 shrink-0 items-center gap-2 rounded-lg px-3 py-2 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 ${activePage === plugin.file ? "bg-secondary text-secondary-foreground" : "text-muted-foreground hover:bg-muted/60 hover:text-foreground"}`}
                  >
                    <Icon className="size-4 shrink-0" />
                    <span className="flex min-w-0 items-center gap-2">
                      <span className="whitespace-nowrap text-sm font-medium">
                        {t(
                          PLUGIN_NAMES[plugin.file.toLowerCase()] ??
                            plugin.file,
                        )}
                      </span>
                      <span className="whitespace-nowrap rounded bg-background px-1.5 py-0.5 text-[11px]">
                        {t(PLUGIN_STATE[plugin.state])}
                      </span>
                    </span>
                  </button>
                );
              })}
            </div>
            {panel.plugins.length === 0 && (
              <p className="px-3 py-2 text-xs leading-relaxed text-muted-foreground">
                {t(
                  panel.connection === "connected"
                    ? "No compiled plugins found"
                    : "Plugin list will appear after connection.",
                )}
              </p>
            )}
          </nav>

          <div
            className="@container/detail min-h-0 min-w-0 flex-1 overflow-y-auto [scrollbar-gutter:stable]"
            ref={contentRef}
            data-testid="plugin-workspace-scroll"
          >
            <div className="w-full space-y-4 p-4">
              {errorBanner}
              <div key={`${activePage}:${panel.connectionIdentity}`}>
                {selectedPlugin ? (
                  <PluginControls
                    panel={panel}
                    plugin={selectedPlugin}
                    refresh={refresh}
                    title={t(
                      PLUGIN_NAMES[selectedPlugin.file.toLowerCase()] ??
                        selectedPlugin.file,
                    )}
                    pending={pending}
                    run={run}
                  />
                ) : (
                  <HostControls
                    onOpenMarket={() => setTab("market")}
                    panel={panel}
                    pending={pending}
                    run={run}
                    refresh={refresh}
                  />
                )}
              </div>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
