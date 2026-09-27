import { useEffect, useRef, useState } from "react";
import { RefreshCw, Power, ArrowRight, Check, Store } from "lucide-react";
import { Button } from "@/components/ui/button";
import { t } from "@/lib/i18n";
import { toolkitClient, type PluginPanel } from "@/lib/tauri/toolkit-client";
import { HostLoadingControls } from "./host-loading-controls";
import { ReleaseControls } from "./release-controls";
import { RELEASE_GROUPS } from "./release-control-groups";
const CONNECTION = {
  notRequested: "Plugin mode is off",
  connected: "Connected",
  unavailable: "Waiting for UE Tools",
  busy: "Plugin busy",
  error: "Plugin connection failed",
} as const;
export function HostControls({
  panel,
  pending,
  run,
  refresh,
  onOpenMarket,
}: {
  panel: PluginPanel;
  pending: boolean;
  run: (action: () => Promise<unknown>) => Promise<void>;
  refresh: () => Promise<void>;
  onOpenMarket: () => void;
}) {
  const [notice, setNotice] = useState<string | null>(null);
  const alive = useRef(false);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const connected = panel.connection === "connected";
  const guidance = panel.collectorActive
    ? "Stop collection before changing plugin lifecycle state."
    : panel.mode !== "plugin"
      ? "Choose Plugin mode in the home page data source selector first."
      : !panel.componentsReady
        ? "Install the required components from the Mod Market first."
        : connected
          ? "Select a plugin above to load it or adjust its features."
          : panel.loadingMethod === "proxy"
            ? "Close the game, load the host to deploy the proxy, then restart the game and check status."
            : "Start the game, then load the host. Approve UAC if prompted.";
  return (
    <div
      className="flex min-w-0 flex-col gap-4"
      data-testid="host-control-page"
    >
      <section
        aria-label={t("Getting started")}
        className="rounded-xl border bg-muted/25 px-4 py-3"
      >
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
          <h2 className="text-sm font-semibold">{t("Getting started")}</h2>
          <Button
            size="sm"
            variant="outline"
            disabled={pending}
            onClick={onOpenMarket}
          >
            <Store />
            {t("Open Mod Market")}
            <ArrowRight />
          </Button>
        </div>
        <ol className="grid gap-3 text-xs @min-[42rem]/detail:grid-cols-3">
          {(
            [
              [
                "Install components",
                "Get the host, Loader, driver and plugins from the market.",
                panel.componentsReady,
              ],
              [
                "Load host",
                "Choose a loading method below and connect to the game.",
                connected,
              ],
              [
                "Use plugins",
                "Select a plugin above to load it or adjust its features.",
                false,
              ],
            ] as const
          ).map(([title, description, done], index) => (
            <li key={title} className="flex items-start gap-2">
              <span
                className={`flex size-5 shrink-0 items-center justify-center rounded-full text-[11px] ${done ? "bg-primary text-primary-foreground" : "border bg-background"}`}
              >
                {done ? (
                  <Check className="size-3" aria-label={t("Ready")} />
                ) : (
                  index + 1
                )}
              </span>
              <div>
                <p className="font-medium">{t(title)}</p>
                <p className="mt-1 leading-relaxed text-muted-foreground">
                  {t(description)}
                </p>
              </div>
            </li>
          ))}
        </ol>
      </section>
      <div className="grid items-start gap-4 @min-[46rem]/detail:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]">
        <section
          className="min-w-0 rounded-xl border bg-card"
          aria-label={t("Plugin host")}
        >
          <header className="flex flex-wrap items-center justify-between gap-2 border-b px-4 py-3">
            <h2 className="text-sm font-semibold">{t("Plugin host")}</h2>
            <span
              role="status"
              className="flex items-center gap-2 text-xs font-medium"
            >
              <span
                className={`size-2 rounded-full ${connected ? "bg-[var(--console-success)]" : "bg-muted-foreground"}`}
              />
              {t(CONNECTION[panel.connection])}
            </span>
          </header>
          <div className="space-y-3 p-4">
            <HostLoadingControls panel={panel} pending={pending} run={run} />
            <p
              id="host-load-guidance"
              className="border-l-2 border-primary pl-3 text-sm leading-relaxed"
            >
              {t(guidance)}
            </p>
            <div className="flex flex-wrap items-center justify-end gap-2">
              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={pending}
                  onClick={() => void refresh()}
                >
                  <RefreshCw />
                  {t("Check status")}
                </Button>
                <Button
                  size="sm"
                  aria-describedby="host-load-guidance"
                  disabled={
                    pending ||
                    panel.collectorActive ||
                    connected ||
                    panel.mode !== "plugin" ||
                    !panel.componentsReady
                  }
                  onClick={() => {
                    setNotice(null);
                    void run(async () => {
                      const result = await toolkitClient.launchHost();
                      if (alive.current)
                        setNotice(
                          result.outcome === "proxyDeployed"
                            ? "Proxy deployment is ready. Restart the game, then check host status."
                            : "Host connection confirmed.",
                        );
                    });
                  }}
                >
                  <Power />
                  {t("Load host")}
                </Button>
              </div>
            </div>
            {notice && (
              <p role="status" className="text-sm font-medium">
                {t(notice)}
              </p>
            )}
          </div>
        </section>
        <ReleaseControls
          panel={panel}
          pending={pending}
          run={run}
          group={RELEASE_GROUPS.find((group) => group.id === "host")!}
        />
      </div>
    </div>
  );
}
