import {
  PageHeader,
  PageTitle,
  PageDescription,
  PageScrollArea,
} from "@/components/nte/page-layout";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  Check,
  Cpu,
  Server,
  Settings2,
  Store,
  TriangleAlert,
} from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { t, tf, useFrontendLanguage } from "@/lib/i18n";
import { type CommandError, toCommandError } from "@/lib/tauri/command-error";
import type { PluginPanel } from "@/lib/tauri/generated/PluginPanel";
import { toolkitClient } from "@/lib/tauri/toolkit-client";
import { HostControls } from "./host-controls";
import { PluginControls } from "./plugin-controls";
import { usePluginFormCatalog, pluginFormTitle } from "./plugin-form-catalog";
import { PLUGIN_STATE } from "./release-control-groups";
import { useModMarket } from "./use-mod-market";
import { hasModMarketUpdate } from "./mod-market-model";
import { ModMarketPanel } from "./mod-market-panel";
export function ModStudioWorkspace() {
  const language = useFrontendLanguage();
  const entries = usePluginFormCatalog((state) => state.entries);
  const loadForm = usePluginFormCatalog((state) => state.load);
  const [panel, setPanel] = useState<PluginPanel | null>(null);
  const [tab, setTab] = useState<"control" | "market">("control");
  const [selectedPage, setSelectedPage] = useState("host");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);
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
        setError({
          code: "plugin_panel_read_failed",
          messageKey: "Failed to read the plugin control panel.",
          messageArguments: [],
        });
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
  const identity =
    panel?.mode === "plugin" && panel.connection === "connected"
      ? panel.connectionIdentity
      : null;
  const providers = JSON.stringify(
    panel?.plugins
      .filter((plugin) => plugin.state === "loaded")
      .map((plugin) => [plugin.file, plugin.serviceOwner]) ?? [],
  );
  useEffect(() => {
    if (!identity) return;
    void (async () => {
      for (const [file, owner] of JSON.parse(providers) as [
        string,
        string | undefined,
      ][]) {
        if (owner) await loadForm(file, identity, owner);
      }
    })();
  }, [identity, providers, loadForm]);
  const market = useModMarket(refresh);
  const updatesAvailable =
    market.state.status === "ready" &&
    market.state.catalog.mods.some(hasModMarketUpdate);
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
      if (alive.current) setError(toCommandError(e));
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
    const provider = panel?.plugins.find((plugin) => plugin.file === page);
    if (identity && provider?.state === "loaded" && provider.serviceOwner)
      void loadForm(provider.file, identity, provider.serviceOwner, true);
    setError(null);
    contentRef.current?.scrollTo({ top: 0 });
  };
  const errorBanner = error ? (
    <Alert variant="destructive" className="shrink-0">
      <TriangleAlert />
      <AlertTitle>{t("Operation failed")}</AlertTitle>
      <AlertDescription>
        {tf(error.messageKey, error.messageArguments)}{" "}
        <span className="font-mono text-xs">({error.code})</span>
      </AlertDescription>
    </Alert>
  ) : null;
  return (
    <section className="@container/workshop flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <PageHeader className="flex shrink-0 flex-wrap items-center justify-between gap-3">
        <div>
          <PageTitle>{t("Mod Workshop")}</PageTitle>
          <PageDescription>
            {t("Your plugins and in-game features, in one place.")}
          </PageDescription>
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
            {updatesAvailable && (
              <span
                role="img"
                aria-label={t("Updates available")}
                title={t("Updates available")}
                className="size-2 rounded-full bg-destructive"
              />
            )}
          </Button>
        </div>
      </PageHeader>
      {tab === "market" ? (
        <PageScrollArea ref={contentRef}>
          <div className="mx-auto w-full max-w-7xl space-y-5">
            {errorBanner}
            <ModMarketPanel market={market} />
          </div>
        </PageScrollArea>
      ) : panel === null ? (
        <div className="space-y-4 p-5">
          {errorBanner}
          <Skeleton className="h-40" />
        </div>
      ) : (
        <div
          className="flex min-h-0 flex-1 flex-col @min-[44rem]/workshop:flex-row"
          data-testid="plugin-split-view"
        >
          <nav
            aria-label={t("Plugin control pages")}
            className="flex max-h-48 shrink-0 flex-wrap content-start gap-1 overflow-y-auto border-b bg-muted/20 p-3 @min-[44rem]/workshop:max-h-none @min-[44rem]/workshop:w-44 @min-[44rem]/workshop:flex-col @min-[44rem]/workshop:flex-nowrap @min-[44rem]/workshop:border-r @min-[44rem]/workshop:border-b-0"
          >
            <Button
              variant={activePage === "host" ? "secondary" : "ghost"}
              className="shrink-0 justify-start @min-[44rem]/workshop:w-full"
              aria-current={activePage === "host" ? "page" : undefined}
              disabled={pending}
              onClick={() => selectPage("host")}
            >
              <Server />
              {t("Connection settings")}
            </Button>
            <div className="contents">
              {panel.plugins.map((plugin) => {
                return (
                  <button
                    type="button"
                    key={plugin.file}
                    disabled={pending}
                    aria-current={
                      activePage === plugin.file ? "page" : undefined
                    }
                    onClick={() => selectPage(plugin.file)}
                    className={`flex min-w-0 max-w-full shrink-0 items-center gap-2 rounded-lg px-3 py-2.5 @min-[44rem]/workshop:w-full text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 ${activePage === plugin.file ? "bg-secondary text-secondary-foreground" : "text-muted-foreground hover:bg-muted/60 hover:text-foreground"}`}
                  >
                    <Cpu className="size-4 shrink-0" />
                    <span className="flex min-w-0 flex-1 items-center gap-2">
                      <span className="truncate text-sm font-medium">
                        {pluginFormTitle(
                          entries[plugin.file]?.snapshot,
                          language,
                        )}
                      </span>
                      <span
                        role="img"
                        aria-label={t(PLUGIN_STATE[plugin.state])}
                        title={t(PLUGIN_STATE[plugin.state])}
                        className={`ml-auto size-1.5 shrink-0 rounded-full ${plugin.state === "loaded" ? "bg-success" : plugin.state === "failed" ? "bg-destructive" : "bg-muted-foreground"}`}
                      />
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

          <PageScrollArea
            className="@container/detail"
            ref={contentRef}
            data-testid="plugin-workspace-scroll"
          >
            <div className="mx-auto w-full max-w-6xl space-y-4">
              {errorBanner}
              <div key={`${activePage}:${panel.connectionIdentity}`}>
                {selectedPlugin ? (
                  <PluginControls
                    panel={panel}
                    plugin={selectedPlugin}
                    title={pluginFormTitle(
                      entries[selectedPlugin.file]?.snapshot,
                      language,
                    )}
                    pending={pending}
                    run={run}
                  />
                ) : (
                  <HostControls
                    refresh={refresh}
                    onOpenMarket={() => setTab("market")}
                    panel={panel}
                    pending={pending}
                    run={run}
                  />
                )}
              </div>
            </div>
          </PageScrollArea>
        </div>
      )}
    </section>
  );
}
