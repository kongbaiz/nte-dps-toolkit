import type { HostAction } from "@/lib/tauri/generated/HostAction";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { t } from "@/lib/i18n";
import { invokeCommand } from "@/lib/tauri/command-error";
import type { PluginPanel } from "@/lib/tauri/generated/PluginPanel";
import type { ReleaseResult } from "@/lib/tauri/generated/ReleaseResult";
import { NativeResponse, OperationConfirmation } from "./native-operation";

export function HostRuntimeControls({
  panel,
  pending,
  run,
}: {
  panel: PluginPanel;
  pending: boolean;
  run: (action: () => Promise<unknown>) => Promise<void>;
}) {
  const [result, setResult] = useState<ReleaseResult | null>(null);
  const [confirm, setConfirm] = useState(false);
  const generation = useRef({ value: 0 });
  useEffect(() => {
    const lifetime = generation.current;
    generation.current.value++;
    setResult(null);
    setConfirm(false);
    return () => {
      lifetime.value++;
    };
  }, [panel.connectionIdentity]);
  const execute = (action: HostAction, confirmed = false) => {
    const ticket = generation.current.value;
    void run(async () => {
      try {
        const next = await invokeCommand<ReleaseResult>("host_action", {
          action,
          expectedIdentity: panel.connectionIdentity,
          confirmed,
        });
        if (ticket === generation.current.value) setResult(next);
      } finally {
        if (ticket === generation.current.value) setConfirm(false);
      }
    });
  };
  const disabled =
    pending || panel.mode !== "plugin" || panel.connection !== "connected";
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          disabled={disabled || !panel.capabilities.includes(2)}
          onClick={() => execute("hostStatus")}
        >
          {t("Host status")}
        </Button>
        <Button
          variant="outline"
          disabled={
            disabled ||
            panel.collectorActive ||
            !panel.capabilities.includes(11)
          }
          onClick={() => setConfirm(true)}
        >
          {t("Unload host")}
        </Button>
      </div>
      {result && <NativeResponse result={result} />}
      <OperationConfirmation
        message={
          confirm
            ? t(
                "This operation can reset data, cancel work or stop the plugin host. Packet capture will not be started as a fallback.",
              )
            : null
        }
        pending={pending}
        cancel={() => setConfirm(false)}
        confirm={() => execute("shutdown", true)}
      />
    </div>
  );
}
