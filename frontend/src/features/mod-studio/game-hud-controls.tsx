import { useEffect, useRef, useState } from "react";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { t } from "@/lib/i18n";
import {
  parseHudOptions,
  releasePluginClient,
} from "@/lib/tauri/release-plugin-client";
import type { PluginPanel } from "@/lib/tauri/toolkit-client";

const FEATURES = [
  [1, "Skill cooldown HUD"],
  [2, "Enemy bars HUD"],
  [4, "Skill ready cue"],
  [8, "HUD health values"],
  [16, "HUD unbalance values"],
] as const;

export function GameHudControls({
  panel,
  pending,
  run,
}: {
  panel: PluginPanel;
  pending: boolean;
  run: (action: () => Promise<unknown>) => Promise<void>;
}) {
  const identity = panel.connectionIdentity;
  const available =
    panel.mode === "plugin" &&
    panel.connection === "connected" &&
    panel.capabilities.includes(118) &&
    panel.capabilities.includes(119) &&
    panel.plugins.some(
      (p) =>
        p.file.toLowerCase() === "nte_plugincombat.dll" && p.state === "loaded",
    );
  const [options, setOptions] = useState<number | null>(null);
  const [error, setError] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [busy, setBusy] = useState(false);
  const epoch = useRef({ value: 0 });
  const writing = useRef(false);
  useEffect(() => {
    const lifetime = epoch.current;
    const ticket = ++lifetime.value;
    setOptions(null);
    setError(false);
    setBusy(false);
    writing.current = false;
    if (available && identity) {
      void releasePluginClient
        .execute("hudStatus", identity)
        .then((result) => {
          const next = parseHudOptions(result);
          if (ticket === epoch.current.value) setOptions(next);
        })
        .catch(() => {
          if (ticket === epoch.current.value) setError(true);
        });
    }
    return () => {
      lifetime.value++;
    };
  }, [available, identity, refresh]);
  const configure = (mask: number) => {
    if (
      !available ||
      !identity ||
      pending ||
      writing.current ||
      options === null
    )
      return;
    writing.current = true;
    setBusy(true);
    setError(false);
    const ticket = ++epoch.current.value;
    void run(async () => {
      try {
        const next = parseHudOptions(
          await releasePluginClient.execute("hudConfigure", identity, mask),
        );
        if (ticket === epoch.current.value) setOptions(next);
      } catch (e) {
        if (ticket === epoch.current.value) {
          setOptions(null);
          setError(true);
        }
        throw e; // Parent shows the stable command error; never retry a mutation.
      } finally {
        if (ticket === epoch.current.value) {
          writing.current = false;
          setBusy(false);
        }
      }
    }).finally(() => {
      if (ticket === epoch.current.value) {
        writing.current = false;
        setBusy(false);
      }
    });
  };
  const disabled = !available || options === null || pending || busy;
  return (
    <Card
      className="min-w-0 gap-0 overflow-hidden rounded-xl py-0 shadow-none"
      data-testid="plugin-hud-settings"
    >
      <CardHeader className="flex flex-row items-start justify-between gap-4 px-5 py-5">
        <div>
          <CardTitle>{t("In-game enhanced HUD")}</CardTitle>
          <p className="mt-1.5 max-w-sm text-xs leading-relaxed text-muted-foreground">
            {t("Independent from desktop DPS and capture.")}
          </p>
        </div>
        <Switch
          aria-label={t("In-game enhanced HUD")}
          checked={options !== null && options !== 0}
          disabled={disabled}
          onCheckedChange={(on) => configure(on ? 31 : 0)}
        />
      </CardHeader>
      <CardContent className="border-t p-0">
        {!available ? (
          <p className="p-5 text-sm text-muted-foreground">
            {t("Update the Release host to control the in-game HUD.")}
          </p>
        ) : (
          <>
            <div className="divide-y">
              {FEATURES.map(([bit, label]) => (
                <label
                  key={bit}
                  className="flex min-h-12 items-center justify-between gap-4 px-5 py-3 text-sm"
                >
                  {t(label)}
                  <Switch
                    aria-label={t(label)}
                    checked={options !== null && (options & bit) !== 0}
                    disabled={disabled}
                    onCheckedChange={(on) => {
                      if (options !== null)
                        configure(on ? options | bit : options & ~bit);
                    }}
                  />
                </label>
              ))}
            </div>
            {error && (
              <p role="alert" className="px-5 py-3 text-sm text-destructive">
                {t("HUD status is unavailable.")}
              </p>
            )}
            <div className="border-t bg-muted/20 px-3 py-2">
              <Button
                size="sm"
                variant="ghost"
                disabled={pending || busy}
                onClick={() => setRefresh((v) => v + 1)}
              >
                {t("Refresh")}
              </Button>
            </div>
          </>
        )}
      </CardContent>
    </Card>
  );
}
