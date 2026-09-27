import { useEffect, useRef, useState } from "react";
import { Cable, Network, Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import { t } from "@/lib/i18n";
import { toolkitClient, type DataMode } from "@/lib/tauri/toolkit-client";
import {
  AlertDialog,
  AlertDialogTrigger,
  AlertDialogBackdrop,
  AlertDialogPortal,
  AlertDialogPopup,
  AlertDialogTitle,
  AlertDialogDescription,
} from "@/components/ui/alert-dialog";

export function DataSourceControl({
  disabled = false,
}: {
  disabled?: boolean;
}) {
  const [mode, setMode] = useState<DataMode | null>(null);
  const [confirmMode, setConfirmMode] = useState<DataMode | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const alive = useRef(false);
  const busy = useRef(false);
  useEffect(() => {
    let active = true;
    alive.current = true;
    setError(null);
    void toolkitClient
      .snapshot()
      .then((panel) => {
        if (active) setMode(panel.mode);
      })
      .catch(() => {
        if (active) setError("Failed to read the plugin control panel.");
      });
    return () => {
      active = false;
      alive.current = false;
    };
  }, [reload]);
  const run = async (action: () => Promise<void>) => {
    if (busy.current || disabled) return;
    busy.current = true;
    setPending(true);
    setError(null);
    try {
      await action();
    } catch {
      if (alive.current) setMode(null);
      if (alive.current)
        setError(
          "Failed to switch data source. Check the current mode before retrying.",
        );
    } finally {
      busy.current = false;
      if (alive.current) setPending(false);
    }
  };
  return (
    <AlertDialog
      open={confirmMode !== null}
      onOpenChange={(open) => {
        if (!open && !pending) setConfirmMode(null);
      }}
    >
      <div
        className="flex flex-wrap items-center gap-2 border-b px-3 py-2"
        aria-label={t("Data source")}
      >
        <span className="text-xs text-muted-foreground">
          {t("Data source")}
        </span>
        <span role="status" className="text-xs font-medium">
          {t(
            mode === "plugin"
              ? "Current mode: Plugin"
              : mode === "packet_capture"
                ? "Current mode: Packet capture"
                : "Reading current mode...",
          )}
        </span>
        <div className="flex gap-1 rounded-lg bg-muted/70 p-1">
          {(["plugin", "packet_capture"] as const).map((value) => (
            <AlertDialogTrigger
              key={value}
              disabled={pending || disabled || mode === null}
              render={
                <Button
                  size="sm"
                  variant={mode === value ? "default" : "ghost"}
                  aria-pressed={mode === value}
                />
              }
              onClick={() => {
                if (mode !== value) setConfirmMode(value);
              }}
            >
              {mode === value ? (
                <Check aria-hidden="true" />
              ) : value === "plugin" ? (
                <Cable />
              ) : (
                <Network />
              )}
              {t(value === "plugin" ? "Plugin mode" : "Packet capture mode")}
            </AlertDialogTrigger>
          ))}
        </div>
        {error && (
          <span role="alert" className="text-xs text-destructive">
            {t(error)}
            <Button
              size="sm"
              variant="ghost"
              disabled={pending}
              onClick={() => setReload((value) => value + 1)}
            >
              {t("Retry")}
            </Button>
          </span>
        )}
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
                      const panel = await toolkitClient.setMode(
                        mode,
                        mode === "plugin",
                      );
                      if (alive.current) setMode(panel.mode);
                      if (alive.current) setConfirmMode(null);
                    });
                }}
              >
                {t("Confirm")}
              </Button>
            </div>
          </AlertDialogPopup>
        </AlertDialogPortal>
      </div>
    </AlertDialog>
  );
}
