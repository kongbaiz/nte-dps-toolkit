import { useEffect, useRef, useState } from "react";
import { RefreshCw, Power, ArrowRight, Store } from "lucide-react";
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
          ? "Choose a plugin to start using its features."
          : panel.loadingMethod === "proxy"
            ? "Close the game, load the host to deploy the proxy, then restart the game and check status."
            : "Start the game, then load the host. Approve UAC if prompted.";
  return (
    <div
      className="flex min-w-0 flex-col gap-4"
      data-testid="host-control-page"
    >
      {!panel.componentsReady && (
        <section
          className="flex flex-wrap items-center justify-between gap-3 rounded-xl border bg-muted/25 p-4"
          aria-label={t("Getting started")}
        >
          <div>
            <h2 className="text-sm font-semibold">{t("Install components")}</h2>
            <p className="mt-1 text-xs text-muted-foreground">
              {t("Get the host, Loader, driver and plugins from the market.")}
            </p>
          </div>
          <Button size="sm" disabled={pending} onClick={onOpenMarket}>
            <Store />
            {t("Open Mod Market")}
            <ArrowRight />
          </Button>
        </section>
      )}
      <div className="space-y-4">
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
            {!connected && (
              <HostLoadingControls panel={panel} pending={pending} run={run} />
            )}
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
                {!connected && (
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
                )}
              </div>
            </div>
            {notice && (
              <p role="status" className="text-sm font-medium">
                {t(notice)}
              </p>
            )}
          </div>
        </section>
        <details className="rounded-xl border bg-card">
          <summary className="cursor-pointer px-4 py-3 text-sm font-medium focus-visible:outline-ring">
            {t("Connection settings and maintenance")}
          </summary>
          <div className="space-y-4 border-t p-4">
            {connected && (
              <HostLoadingControls panel={panel} pending={pending} run={run} />
            )}
            <ReleaseControls
              panel={panel}
              pending={pending}
              run={run}
              group={RELEASE_GROUPS.find((group) => group.id === "host")!}
            />
          </div>
        </details>
      </div>
    </div>
  );
}
