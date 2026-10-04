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
