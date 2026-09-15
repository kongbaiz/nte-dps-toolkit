import { useCallback, useEffect, useRef, useState } from "react";
import {
  Cable,
  Network,
  RefreshCw,
  Settings2,
  Store,
  TriangleAlert,
} from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import {
  AlertDialog,
  AlertDialogBackdrop,
  AlertDialogPortal,
  AlertDialogPopup,
  AlertDialogTitle,
  AlertDialogDescription,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { t, useTranslationRevision } from "@/lib/i18n";
import {
  toolkitClient,
  type DataMode,
  type PluginPanel,
} from "@/lib/tauri/toolkit-client";
import { consoleControlClient } from "@/lib/tauri/console-control-client";
import { ModMarketPanel } from "./mod-market-panel";

const CONNECTION = {
  notRequested: "Plugin mode is off",
  connected: "Connected",
  unavailable: "Waiting for UE Tools",
  busy: "Plugin busy",
  error: "Plugin connection failed",
} as const;
const PLUGIN_STATE = {
  loaded: "Loaded",
  unloaded: "Disabled",
  unload_pending: "Unloading",
  failed: "Load failed",
} as const;

export function ModStudioWorkspace() {
  useTranslationRevision();
  const [panel, setPanel] = useState<PluginPanel | null>(null);
  const [tab, setTab] = useState<"control" | "market">("control");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmMode, setConfirmMode] = useState<DataMode | null>(null);
  const alive = useRef(false);
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
      if (!busy.current) await refresh();
      if (alive.current) timer = setTimeout(() => void poll(), 1000);
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
  return (
    <section className="flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto p-4">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">{t("Mod Workshop")}</h1>
          <p className="text-sm text-muted-foreground">
            {t("Data sources and compiled plugins")}
          </p>
        </div>
        <div className="flex gap-2">
          <Button
            variant={tab === "control" ? "secondary" : "outline"}
            onClick={() => setTab("control")}
          >
            <Settings2 data-icon="inline-start" />
            {t("Control panel")}
          </Button>
          <Button
            variant={tab === "market" ? "secondary" : "outline"}
            onClick={() => setTab("market")}
          >
            <Store data-icon="inline-start" />
            {t("Mod Market")}
          </Button>
        </div>
      </header>
      {error && (
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertTitle>{t("Operation failed")}</AlertTitle>
          <AlertDescription>{t(error)}</AlertDescription>
        </Alert>
      )}
      {panel === null ? (
        <Skeleton className="h-40 w-full" />
      ) : (
        <Card>
          <CardHeader>
            <CardTitle>{t("Data source")}</CardTitle>
            <CardDescription>
              {t(
                "Switching modes stops the current collector. Recorded data is retained until the next confirmed start.",
              )}
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            <div className="grid gap-4 md:grid-cols-2">
              <div className="flex gap-3">
                <Network className="size-5 shrink-0" />
                <div>
                  <p className="font-medium">
                    {t("Packet capture mode")}
                    {panel.mode === "packet_capture" && (
                      <Badge className="ml-2" variant="secondary">
                        {t("Current")}
                      </Badge>
                    )}
                  </p>
                  <p className="mt-1 text-sm text-muted-foreground">
                    {t(
                      "Default mode. Lower risk; available data is limited and packet-derived results may be inaccurate.",
                    )}
                  </p>
                </div>
              </div>
              <div className="flex gap-3">
                <Cable className="size-5 shrink-0" />
                <div className="flex-1">
                  <label htmlFor="plugin-mode" className="font-medium">
                    {t("Plugin mode")}
                  </label>
                  <p className="mt-1 text-sm text-muted-foreground">
                    {t(
                      "Reads UE Tools directly without packet parsing. More complete native data, with plugin risk; unknown and inferred values stay marked.",
                    )}
                  </p>
                </div>
                <Switch
                  id="plugin-mode"
                  checked={panel.mode === "plugin"}
                  disabled={pending}
                  onCheckedChange={(on) =>
                    setConfirmMode(on ? "plugin" : "packet_capture")
                  }
                />
              </div>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant="outline">{t(CONNECTION[panel.connection])}</Badge>
              <Button
                variant="outline"
                size="sm"
                disabled={pending}
                onClick={() => void run(() => toolkitClient.chooseDirectory())}
              >
                {t("Select installed Toolkit folder")}
              </Button>
              <span className="text-xs text-muted-foreground">
                {t(
                  panel.directorySelected
                    ? "Toolkit folder selected"
                    : "Install the compiled Toolkit package first; do not mix it with the legacy Mod DLL.",
                )}
              </span>
            </div>
          </CardContent>
        </Card>
      )}
      {tab === "market" ? (
        <ModMarketPanel onInstalled={refresh} />
      ) : (
        panel && (
          <>
            <Card>
              <CardHeader>
                <CardTitle>{t("Combat data")}</CardTitle>
                <CardDescription>
                  {t(
                    "Collection uses only the selected data source. Disconnection never starts packet capture automatically.",
                  )}
                </CardDescription>
              </CardHeader>
              <CardContent className="flex flex-col gap-4">
                {panel.combat ? (
                  <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
                    {[
                      [
                        "Total damage",
                        panel.combat.totalDamage.toLocaleString(undefined, {
                          maximumFractionDigits: 6,
                        }),
                      ],
                      ["Hits", panel.combat.hits],
                      ["Inferred", panel.combat.inferred.toLocaleString()],
                      ["Unknown", panel.combat.unknown.toLocaleString()],
                    ].map(([label, value]) => (
                      <div key={label}>
                        <p className="text-xs text-muted-foreground">
                          {t(label)}
                        </p>
                        <p className="mt-1 font-mono text-xl tabular-nums">
                          {value}
                        </p>
                      </div>
                    ))}
                  </div>
                ) : (
                  <p className="text-sm text-muted-foreground">
                    {t(
                      panel.mode === "plugin"
                        ? "No live plugin data. Check the game, host version and plugin loading state."
                        : "Packet capture does not load or query game plugins.",
                    )}
                  </p>
                )}
                <div>
                  <Button
                    disabled={
                      pending ||
                      (panel.mode === "plugin" &&
                        panel.connection !== "connected")
                    }
                    onClick={() =>
                      void run(() =>
                        consoleControlClient.execute("toggle-capture"),
                      )
                    }
                  >
                    {t("Start / stop collection")}
                  </Button>
                </div>
              </CardContent>
            </Card>
            <Card>
              <CardHeader>
                <CardTitle>{t("Plugins")}</CardTitle>
                <CardDescription>
                  {t(
                    "Loading state is reported by the host, not inferred from files on disk.",
                  )}
                </CardDescription>
              </CardHeader>
              <CardContent className="flex flex-col gap-3">
                {panel.plugins.length === 0 && (
                  <p className="text-sm text-muted-foreground">
                    {t(
                      panel.connection === "connected"
                        ? "No compiled plugins found"
                        : "Plugin loading state is unavailable",
                    )}
                  </p>
                )}
                {panel.plugins.map((plugin) => (
                  <div
                    key={plugin.file}
                    className="flex flex-wrap items-center justify-between gap-3 rounded-md border p-3"
                  >
                    <div>
                      <p className="font-mono text-sm">{plugin.file}</p>
                      <Badge className="mt-1" variant="outline">
                        {t(PLUGIN_STATE[plugin.state])}
                      </Badge>
                    </div>
                    <div className="flex gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={
                          pending ||
                          panel.combat?.capturing ||
                          plugin.state === "unload_pending"
                        }
                        onClick={() =>
                          void run(() =>
                            toolkitClient.control(
                              plugin.state === "loaded" ? "disable" : "enable",
                              plugin.file,
                            ),
                          )
                        }
                      >
                        {t(plugin.state === "loaded" ? "Disable" : "Enable")}
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={pending || panel.combat?.capturing}
                        onClick={() =>
                          void run(() =>
                            toolkitClient.control("reload", plugin.file),
                          )
                        }
                      >
                        <RefreshCw data-icon="inline-start" />
                        {t("Reload")}
                      </Button>
                    </div>
                  </div>
                ))}
                {panel.operation?.state && (
                  <p className="text-sm text-muted-foreground">
                    {t("Plugin operation")}: {panel.operation.state}
                    {panel.operation.error && ` · ${t(panel.operation.error)}`}
                  </p>
                )}
                <div className="flex flex-wrap items-center gap-3">
                  <span className="text-sm">{t("Host logging")}</span>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={pending || panel.connection !== "connected"}
                    onClick={() =>
                      void run(() => toolkitClient.control("logLevel", null, 2))
                    }
                  >
                    {t("Info logging")}
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={pending || panel.connection !== "connected"}
                    onClick={() =>
                      void run(() => toolkitClient.control("logLevel", null, 6))
                    }
                  >
                    {t("Logging off")}
                  </Button>
                </div>
                <p className="text-xs text-muted-foreground">
                  {t(
                    "Legacy script execution and equipment changes are not supported by this Toolkit protocol.",
                  )}
                </p>
              </CardContent>
            </Card>
          </>
        )
      )}
      <AlertDialog
        open={confirmMode !== null}
        onOpenChange={(open) => {
          if (!open && !pending) setConfirmMode(null);
        }}
      >
        <AlertDialogPortal>
          <AlertDialogBackdrop />
          <AlertDialogPopup className="left-1/2 top-1/2 flex w-[min(90vw,30rem)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-xl border bg-background p-6 shadow-lg">
            <AlertDialogTitle>
              {t(
                confirmMode === "plugin"
                  ? "Enable plugin mode?"
                  : "Return to packet capture?",
              )}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t(
                confirmMode === "plugin"
                  ? "Plugins run inside the game and carry compatibility and account risks. Switching stops current collection; no plugin data is claimed until the host connects. Continue only if you accept these risks."
                  : "Stop plugin collection and use packet capture. This does not unload the DLL from the game; close the game to remove it completely.",
              )}
            </AlertDialogDescription>
            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                disabled={pending}
                onClick={() => setConfirmMode(null)}
              >
                {t("Cancel")}
              </Button>
              <Button
                disabled={pending}
                onClick={() => {
                  const mode = confirmMode;
                  if (mode)
                    void run(async () => {
                      await toolkitClient.setMode(mode, mode === "plugin");
                      if (alive.current) setConfirmMode(null);
                    });
                }}
              >
                {t("Confirm")}
              </Button>
            </div>
          </AlertDialogPopup>
        </AlertDialogPortal>
      </AlertDialog>
    </section>
  );
}
