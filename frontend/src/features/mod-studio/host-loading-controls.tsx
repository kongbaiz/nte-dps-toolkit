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
            ? "Proxy loading safely deploys to the automatically detected game folder. Restart the game after deployment."
            : "Loader uses the managed package and requests UAC only when needed.",
        )}
      </p>
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
