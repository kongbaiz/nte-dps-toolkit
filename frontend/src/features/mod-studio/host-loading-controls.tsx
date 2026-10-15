import { useEffect, useRef, useState } from "react";
import {
  AlertDialog,
  AlertDialogPortal,
  AlertDialogBackdrop,
  AlertDialogPopup,
  AlertDialogTitle,
  AlertDialogDescription,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { t } from "@/lib/i18n";
import { toolkitClient, type PluginPanel } from "@/lib/tauri/toolkit-client";

export function HostLoadingControls({
  panel,
  pending,
  run,
}: {
  panel: PluginPanel;
  pending: boolean;
  run: (action: () => Promise<unknown>) => Promise<void>;
}) {
  const [confirmRemoval, setConfirmRemoval] = useState(false);
  const [removed, setRemoved] = useState(false);
  const alive = useRef(false);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    setRemoved(false);
    setConfirmRemoval(false);
  }, [panel.gameDirectory, panel.loadingMethod]);
  return (
    <section className="space-y-2" aria-label={t("Host loading method")}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-sm font-medium">{t("Host loading method")}</h3>
        <div className="flex flex-wrap gap-1">
          {(["proxy", "loader"] as const).map((method) => (
            <Button
              key={method}
              size="sm"
              variant={panel.loadingMethod === method ? "default" : "outline"}
              aria-pressed={panel.loadingMethod === method}
              disabled={pending}
              onClick={() => {
                if (panel.loadingMethod !== method)
                  void run(() => toolkitClient.setLoadingMethod(method));
              }}
            >
              {t(method === "proxy" ? "Proxy loading" : "Loader loading")}
            </Button>
          ))}
        </div>
      </div>
      <p className="text-xs leading-relaxed text-muted-foreground">
        {t(
          panel.loadingMethod === "proxy"
            ? "Proxy loading deploys to your selected game directory, or automatically detects it when no directory is selected. Restart the game after deployment."
            : "Loader uses the managed package and requests UAC only when needed.",
        )}
      </p>
      {panel.loadingMethod === "proxy" && (
        <div className="space-y-2 rounded-lg border p-3">
          <p className="text-sm font-medium">{t("Game directory")}</p>
          <p className="break-all text-xs text-muted-foreground">
            {panel.gameDirectory ?? t("Automatic detection")}
          </p>
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={pending || panel.collectorActive}
              onClick={() =>
                void run(() => toolkitClient.selectGameDirectory())
              }
            >
              {t("Select game directory")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={
                pending || panel.collectorActive || panel.gameDirectory === null
              }
              onClick={() => void run(() => toolkitClient.clearGameDirectory())}
            >
              {t("Use automatic detection")}
            </Button>
          </div>
          <Button
            size="sm"
            variant="outline"
            disabled={
              pending ||
              panel.collectorActive ||
              panel.connection === "connected"
            }
            onClick={() => {
              setRemoved(false);
              setConfirmRemoval(true);
            }}
          >
            {t("Uninstall proxy")}
          </Button>
          {removed && (
            <p role="status" className="text-xs">
              {t(
                "Proxy removed. Managed downloads and unrelated game files were preserved.",
              )}
            </p>
          )}
          <AlertDialog
            open={confirmRemoval}
            onOpenChange={(open) => {
              if (!pending) setConfirmRemoval(open);
            }}
          >
            <AlertDialogPortal>
              <AlertDialogBackdrop />
              <AlertDialogPopup className="left-1/2 top-1/2 flex w-[min(90vw,30rem)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-xl border bg-background p-6 shadow-lg">
                <AlertDialogTitle>{t("Uninstall proxy")}</AlertDialogTitle>
                <AlertDialogDescription>
                  {t(
                    "Close the game first. Remove only verified proxy files from this game directory. Keep downloaded components, settings and unrelated files.",
                  )}
                </AlertDialogDescription>
                <p className="break-all text-xs">
                  {panel.gameDirectory ?? t("Automatic detection")}
                </p>
                <div className="flex justify-end gap-2">
                  <Button
                    variant="outline"
                    disabled={pending}
                    onClick={() => setConfirmRemoval(false)}
                  >
                    {t("Cancel")}
                  </Button>
                  <Button
                    disabled={pending}
                    onClick={() =>
                      void run(async () => {
                        try {
                          await toolkitClient.uninstallProxy(true);
                          if (alive.current) setRemoved(true);
                        } finally {
                          if (alive.current) setConfirmRemoval(false);
                        }
                      })
                    }
                  >
                    {t("Confirm")}
                  </Button>
                </div>
              </AlertDialogPopup>
            </AlertDialogPortal>
          </AlertDialog>
          <p className="text-xs text-muted-foreground">
            {t(
              "Select the installation folder or the Win64 folder containing HTGame.exe. Selecting a directory does not deploy files.",
            )}
          </p>
        </div>
      )}
      <details className="text-xs text-muted-foreground">
        <summary className="cursor-pointer py-1 focus-visible:outline-ring">
          {t("Loading notes")}
        </summary>
        <p className="mt-1 leading-relaxed">
          {t(
            "All plugin-mode components are downloaded from the Mod Market into the managed mods directory.",
          )}
        </p>
        <p className="mt-1 leading-relaxed">
          {t(
            "Changing this preference does not unload the running host or remove an installed proxy.",
          )}
        </p>
      </details>
    </section>
  );
}
