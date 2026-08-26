import { Keyboard, MousePointer2, TriangleAlert, X } from "lucide-react";
import { useEffect, useState } from "react";

import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { useSettings } from "@/features/settings/use-settings";
import { t, tf } from "@/lib/i18n";
import {
  formatHotkeyBinding,
  type CaptureSettingsInput,
  type GlobalHotkeyActionId,
  type HotkeyBinding,
  type SettingsSnapshot,
} from "@/lib/tauri/settings-contract";

import { bindingFromKeyboardEvent } from "./shortcut-binding";

const HOTKEY_ACTIONS: ReadonlyArray<{
  id: GlobalHotkeyActionId;
  labelKey: string;
}> = [
  { id: "capture", labelKey: "Start / Stop Capture" },
  { id: "reset", labelKey: "Reset Session" },
  { id: "hud", labelKey: "Toggle Combat HUD" },
  { id: "new-round", labelKey: "New Round" },
];
export function ShortcutsPage() {
  const settings = useSettings();

  return (
    <section className="flex min-w-0 flex-1 flex-col overflow-hidden p-3">
      <div className="min-h-0 flex-1 overflow-y-auto p-px pr-1 pb-1">
        {settings.mutationError ? (
          <Alert className="mb-3" variant="destructive">
            <TriangleAlert aria-hidden="true" />
            <AlertTitle>{t("Setting was not saved")}</AlertTitle>
            <AlertDescription>
              {tf(
                settings.mutationError.messageKey,
                settings.mutationError.messageArguments,
              )}
            </AlertDescription>
            <AlertAction>
              <Button
                aria-label={t("Dismiss")}
                size="icon-xs"
                variant="ghost"
                onClick={settings.clearMutationError}
              >
                <X aria-hidden="true" />
              </Button>
            </AlertAction>
          </Alert>
        ) : null}
        {settings.state.status === "loading" ? (
          <ShortcutsLoading />
        ) : settings.state.status === "error" ? (
          <Alert variant="destructive">
            <TriangleAlert aria-hidden="true" />
            <AlertTitle>{t("Settings could not be loaded")}</AlertTitle>
            <AlertDescription>
              {tf(
                settings.state.error.messageKey,
                settings.state.error.messageArguments,
              )}
            </AlertDescription>
            <AlertAction>
              <Button
                size="sm"
                variant="outline"
                onClick={() => void settings.refresh()}
              >
                {t("Retry")}
              </Button>
            </AlertAction>
          </Alert>
        ) : (
          <div className="mx-auto flex w-full max-w-4xl flex-col gap-3">
            <GlobalHotkeysCard
              snapshot={settings.state.snapshot}
              pending={settings.pendingAction !== null}
              onSetEnabled={settings.setHotkeysEnabled}
              onSetBinding={settings.setHotkeyBinding}
            />
            <PassthroughHotkeyCard
              snapshot={settings.state.snapshot}
              pending={settings.pendingAction !== null}
              onSetCapture={settings.setCapture}
            />
          </div>
        )}
      </div>
    </section>
  );
}

function GlobalHotkeysCard({
  snapshot,
  pending,
  onSetEnabled,
  onSetBinding,
}: {
  snapshot: SettingsSnapshot;
  pending: boolean;
  onSetEnabled: (enabled: boolean) => Promise<void>;
  onSetBinding: (
    action: GlobalHotkeyActionId,
    binding: HotkeyBinding | null,
  ) => Promise<void>;
}) {
  const [recording, setRecording] = useState<GlobalHotkeyActionId | null>(null);
  const bindings = new Map(
    snapshot.hotkeys.bindings.map((item) => [item.action, item.binding]),
  );
  useEffect(() => {
    if (pending) setRecording(null);
  }, [pending]);

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Keyboard className="size-4" aria-hidden="true" />
          {t("Global shortcuts")}
        </CardTitle>
        <CardDescription>
          {t("Manage application-wide shortcuts from one place.")}
        </CardDescription>
      </CardHeader>
      <CardContent className="divide-y">
        <label className="flex items-center justify-between gap-4 pb-3">
          <span className="text-sm font-medium">
            {t("Enable global hotkeys")}
          </span>
          <Switch
            checked={snapshot.hotkeys.enabled}
            disabled={pending}
            aria-label={t("Enable global hotkeys")}
            onCheckedChange={(enabled) => void onSetEnabled(enabled)}
          />
        </label>
        {HOTKEY_ACTIONS.map((action) => {
          const binding = bindings.get(action.id) ?? null;
          return (
            <div
              className="grid gap-2 py-3 sm:grid-cols-[minmax(9rem,0.6fr)_minmax(0,1fr)_auto] sm:items-center"
              key={action.id}
            >
              <span className="text-sm">{t(action.labelKey)}</span>
              <Button
                className="min-w-0 justify-start font-mono"
                type="button"
                variant="outline"
                disabled={pending || !snapshot.hotkeys.enabled}
                onClick={() => setRecording(action.id)}
                onKeyDown={(event) => {
                  if (recording !== action.id) return;
                  event.preventDefault();
                  const next = bindingFromKeyboardEvent(event);
                  if (event.key === "Escape") {
                    setRecording(null);
                  } else if (next) {
                    setRecording(null);
                    void onSetBinding(action.id, next);
                  }
                }}
              >
                {recording === action.id
                  ? t("Press shortcut...")
                  : (formatHotkeyBinding(binding) ?? t("Disabled"))}
              </Button>
              <Button
                type="button"
                variant="ghost"
                disabled={
                  pending || !snapshot.hotkeys.enabled || binding === null
                }
                onClick={() => void onSetBinding(action.id, null)}
              >
                {t("Disable")}
              </Button>
            </div>
          );
        })}
      </CardContent>
      <CardFooter className="justify-end text-xs text-muted-foreground">
        {t("Click, then press a supported key combination; Esc cancels")}
      </CardFooter>
    </Card>
  );
}

function PassthroughHotkeyCard({
  snapshot,
  pending,
  onSetCapture,
}: {
  snapshot: SettingsSnapshot;
  pending: boolean;
  onSetCapture: (settings: CaptureSettingsInput) => Promise<void>;
}) {
  const [recording, setRecording] = useState(false);
  useEffect(() => {
    if (pending) setRecording(false);
  }, [pending]);

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <MousePointer2 className="size-4" aria-hidden="true" />
          {t("Passthrough Hotkey")}
        </CardTitle>
        <CardDescription>
          {t("Toggle mouse passthrough while the combat HUD is active")}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="flex items-center justify-between gap-4">
          <span className="text-sm font-medium">
            {t("Toggle Mouse Passthrough")}
          </span>
          <Button
            className="min-w-36 justify-start font-mono"
            type="button"
            variant="outline"
            aria-label={t("Passthrough Hotkey")}
            disabled={pending}
            onClick={() => setRecording(true)}
            onKeyDown={(event) => {
              if (!recording) return;
              event.preventDefault();
              if (event.key === "Escape") {
                setRecording(false);
                return;
              }
              const binding = bindingFromKeyboardEvent(event);
              if (!binding) return;
              setRecording(false);
              void onSetCapture({
                ...captureInput(snapshot),
                passthroughHotkey: binding,
              });
            }}
          >
            {recording
              ? t("Press shortcut...")
              : formatHotkeyBinding(snapshot.capture.passthroughHotkey)}
          </Button>
        </div>
      </CardContent>
      <CardFooter className="justify-end text-xs text-muted-foreground">
        {t("Click, then press a supported key combination; Esc cancels")}
      </CardFooter>
    </Card>
  );
}

function captureInput(snapshot: SettingsSnapshot): CaptureSettingsInput {
  const {
    devices: _devices,
    autoRoundIdleSecondsMin: _minimum,
    autoRoundIdleSecondsMax: _maximum,
    dpsTimeRuntime: _runtime,
    ...settings
  } = snapshot.capture;
  return settings;
}

function ShortcutsLoading() {
  return (
    <Card
      className="mx-auto w-full max-w-4xl"
      aria-label={t("Loading Settings")}
    >
      <CardHeader>
        <Skeleton className="h-5 w-36" />
        <Skeleton className="h-4 w-64 max-w-full" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-48 w-full" />
      </CardContent>
    </Card>
  );
}
