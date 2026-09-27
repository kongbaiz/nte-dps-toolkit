import { useEffect, useRef, useState } from "react";
import { Cable, ChevronDown, AlertTriangle } from "lucide-react";
import type { ReleaseControlGroup } from "./release-control-groups";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  AlertDialog,
  AlertDialogBackdrop,
  AlertDialogPortal,
  AlertDialogPopup,
  AlertDialogTitle,
  AlertDialogDescription,
} from "@/components/ui/alert-dialog";
import { t } from "@/lib/i18n";
import type { PluginPanel } from "@/lib/tauri/toolkit-client";
import {
  RELEASE_ACTIONS,
  releasePluginClient,
  requiresIdle,
  requiresConfirmation,
  type ReleaseAction,
  type ReleaseResult,
} from "@/lib/tauri/release-plugin-client";

const ACTION_VERBS: Partial<Record<ReleaseAction, string>> = {
  combatReport: "Preview",
  prebattleSkillUnlock: "Unlock",
  userSnapshot: "Preview",
  combatExport: "Export",
  userExport: "Export",
  runtimeRefresh: "Refresh",
  radarRefresh: "Refresh",
  userRefresh: "Refresh",
  combatReset: "Reset",
  traceClear: "Clear",
  shutdown: "Unload host",
  userCancel: "Cancel",
  networkFlush: "Flush",
};

export function ReleaseControls({
  panel,
  pending,
  run,
  group,
}: {
  group: ReleaseControlGroup;
  panel: PluginPanel;
  pending: boolean;
  run: (action: () => Promise<unknown>) => Promise<void>;
}) {
  const [result, setResult] = useState<ReleaseResult | null>(null);
  const [confirmation, setConfirmation] = useState<ReleaseAction | null>(null);
  const epoch = useRef({ generation: 0 });
  useEffect(() => {
    const lifetime = epoch.current;
    lifetime.generation++;
    setResult(null);
    setConfirmation(null);
    return () => {
      lifetime.generation++;
    };
  }, [panel.connectionIdentity, group.id]);
  const invoke = (
    action: ReleaseAction,
    value: number | null = null,
    confirmed = false,
  ) => {
    const identity = panel.connectionIdentity;
    const actions: readonly ReleaseAction[] = [
      ...group.actions,
      ...group.advanced,
    ];
    if (!identity || !actions.includes(action)) return;
    const ticket = epoch.current.generation;
    void run(async () => {
      try {
        const next = await releasePluginClient.execute(
          action,
          identity,
          value,
          confirmed,
        );
        if (epoch.current.generation === ticket) setResult(next);
      } finally {
        // Let the parent error banner remain visible after a failed operation.
        // Do not leave a retry-looking confirmation dialog over the error.
        if (epoch.current.generation === ticket) setConfirmation(null);
      }
    });
  };
  const loaded =
    group.pluginFile === null ||
    panel.plugins.some(
      (plugin) =>
        plugin.file.toLowerCase() === group.pluginFile.toLowerCase() &&
        plugin.state === "loaded",
    );
  const connected = panel.mode === "plugin" && panel.connection === "connected";
  const actionButton = (action: ReleaseAction, advanced = false) => {
    const descriptor = RELEASE_ACTIONS.find((a) => a[0] === action);
    if (!descriptor) return null;
    const [, command, label] = descriptor;
    const supported = panel.capabilities.includes(command) && loaded;
    const disabled =
      pending ||
      !connected ||
      !supported ||
      (requiresIdle(action) && panel.collectorActive);
    if (action === "networkEnable" || action === "traceEnable")
      return (
        <div
          key={action}
          className="flex min-h-12 flex-wrap items-center justify-between gap-3 px-5 py-3"
        >
          <span className="text-sm">{t(label)}</span>
          <div className="flex shrink-0 gap-1">
            {[true, false].map((enabled) => (
              <Button
                key={String(enabled)}
                size="sm"
                variant="outline"
                disabled={disabled}
                onClick={() => invoke(action, enabled ? 1 : 0)}
              >
                {t(enabled ? "Enable" : "Disable")}
              </Button>
            ))}
          </div>
        </div>
      );
    return (
      <div
        key={action}
        className="flex min-h-12 items-center justify-between gap-4 px-5 py-2.5"
      >
        <span className="min-w-0 flex-1 text-sm leading-5">{t(label)}</span>
        <Button
          size="sm"
          className={`min-w-16 shrink-0 ${advanced && requiresConfirmation(action) ? "text-destructive" : ""}`}
          variant="outline"
          disabled={disabled}
          aria-label={t(label)}
          title={
            !supported
              ? t("This feature is not supported by the connected plugin.")
              : undefined
          }
          onClick={() =>
            requiresConfirmation(action)
              ? setConfirmation(action)
              : invoke(action)
          }
        >
          {t(ACTION_VERBS[action] ?? "Inspect")}
        </Button>
      </div>
    );
  };
  return (
    <Card className="min-w-0 shrink-0 gap-0 rounded-xl py-0 shadow-none">
      <div data-testid={`release-controls-${group.id}`}>
        <div className="border-b px-4 py-3">
          <h2 className="text-sm font-semibold">{t(group.label)}</h2>
          {group.id !== "host" && (
            <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
              {t(group.description)}
            </p>
          )}
        </div>
        {connected ? (
          <>
            <div className="divide-y">
              {group.actions.map((action) => actionButton(action))}
            </div>
            {group.advanced.length > 0 && (
              <details key={group.id} className="group border-t">
                <summary className="flex cursor-pointer list-none items-center gap-2 px-5 py-3 text-xs font-medium text-muted-foreground [&::-webkit-details-marker]:hidden">
                  <AlertTriangle className="size-3.5" />
                  {t("Lifecycle operations")}
                  <ChevronDown className="ml-auto size-4 transition-transform group-open:rotate-180" />
                </summary>
                <div className="divide-y border-t">
                  {group.advanced.map((action) => actionButton(action, true))}
                </div>
              </details>
            )}
            <p className="border-t bg-muted/20 px-5 py-3 text-[11px] leading-relaxed text-muted-foreground">
              {t(
                group.id === "host"
                  ? "Unloading stops host services. Exit the game to fully remove the host DLL."
                  : "After submitting an asynchronous operation, check its operation status before continuing.",
              )}
            </p>
          </>
        ) : (
          <div className="flex flex-col items-center justify-center bg-muted/20 px-4 py-5 text-center">
            <div className="mb-2 rounded-lg border bg-background p-2">
              <Cable className="size-6 text-muted-foreground" />
            </div>
            <h3 className="text-sm font-semibold">
              {t(
                panel.mode === "plugin"
                  ? "Connect the Release host first"
                  : "Plugin controls are off",
              )}
            </h3>
            <p className="mt-2 max-w-sm text-sm leading-relaxed text-muted-foreground">
              {t(
                panel.mode === "plugin"
                  ? "Download the required components from the Mod Market, then load the host."
                  : "Switch the data source to plugin mode to use native controls.",
              )}
            </p>
          </div>
        )}
      </div>
      {result && (
        <details open className="border-t bg-muted/20 px-5 py-4">
          <summary className="cursor-pointer text-sm font-medium">
            {t("Native response")}{" "}
            <span className="ml-2 font-mono text-xs font-normal text-muted-foreground">
              {result.totalBytes} B
              {result.truncated ? ` · ${t("Preview truncated")}` : ""}
            </span>
          </summary>
          <pre className="mt-3 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-lg border bg-background p-3 text-xs">
            {result.preview}
          </pre>
          <p className="mt-2 text-xs text-muted-foreground">
            {t("Preview only. Export the full snapshot when needed.")}
          </p>
        </details>
      )}
      <AlertDialog
        open={confirmation !== null}
        onOpenChange={(open) => {
          if (!open && !pending) setConfirmation(null);
        }}
      >
        <AlertDialogPortal>
          <AlertDialogBackdrop />
          <AlertDialogPopup className="left-1/2 top-1/2 flex w-[min(90vw,30rem)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-xl border bg-background p-6 shadow-lg">
            <AlertDialogTitle>{t("Confirm plugin operation")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t(
                confirmation === "prebattleSkillUnlock"
                  ? "Remove only the observed prebattle skill blocker from the local equipped team. Cooldown and energy limits remain unchanged. After submitting, check the unlock status; do not repeat while queued or verifying."
                  : "This operation can reset data, cancel work or stop the plugin host. Packet capture will not be started as a fallback.",
              )}
            </AlertDialogDescription>
            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                disabled={pending}
                onClick={() => setConfirmation(null)}
              >
                {t("Cancel")}
              </Button>
              <Button
                disabled={pending}
                onClick={() => {
                  if (confirmation) invoke(confirmation, null, true);
                }}
              >
                {t("Confirm")}
              </Button>
            </div>
          </AlertDialogPopup>
        </AlertDialogPortal>
      </AlertDialog>
    </Card>
  );
}
