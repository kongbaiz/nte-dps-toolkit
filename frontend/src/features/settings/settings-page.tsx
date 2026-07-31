import { useEffect, useState, type DragEvent, type KeyboardEvent } from "react";
import {
  ArrowDown,
  ArrowUp,
  EyeOff,
  GripVertical,
  MonitorUp,
  Pin,
  RefreshCw,
  SlidersHorizontal,
  TriangleAlert,
  X,
} from "lucide-react";

import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { t, tf } from "@/lib/i18n";
import {
  isHudModuleId,
  type SettingsCommandError,
  type SettingsSnapshot,
} from "@/lib/tauri/settings-contract";
import type { HudModuleId } from "@/lib/tauri/technical-contract";
import { cn } from "@/lib/utils";

import {
  PrimarySettingsColumn,
  SecondarySettingsSections,
  type SettingsCatalogActions,
} from "./settings-catalog";
import {
  adjacentHudModuleMove,
  hudModuleVisible,
  settingsSectionPending,
} from "./settings-view-model";
import { useSettings } from "./use-settings";

const MODULE_LABEL_KEYS: Record<HudModuleId, string> = {
  title: "Title",
  summary: "Summary",
  status: "Status",
  characters: "Character Ranking",
  timeline: "Curve",
};

export function SettingsPage() {
  const settings = useSettings();
  const catalogActions: SettingsCatalogActions = settings;

  return (
    <section className="flex min-w-0 flex-1 flex-col overflow-hidden p-3">
      {settings.mutationError ? (
        <MutationError
          error={settings.mutationError}
          onDismiss={settings.clearMutationError}
        />
      ) : null}
      <header className="flex shrink-0 flex-wrap items-start justify-between gap-3 px-1 py-1">
        <h1 className="font-heading text-xl font-medium">{t("Settings")}</h1>
        <Button
          variant="outline"
          disabled={settings.pendingAction !== null}
          onClick={() => void settings.refresh()}
        >
          <RefreshCw
            className={cn(
              "size-4",
              settings.state.status === "loading" && "animate-spin",
            )}
            aria-hidden="true"
          />
          {t("Refresh")}
        </Button>
      </header>

      <div className="mt-3 min-h-0 flex-1 overflow-y-auto p-px pr-1 pb-1">
        {settings.state.status === "loading" ? (
          <SettingsLoading />
        ) : settings.state.status === "error" ? (
          <SettingsError
            error={settings.state.error}
            onRetry={settings.refresh}
          />
        ) : (
          <div className="grid grid-cols-1 items-start gap-3 min-[1100px]:grid-cols-2">
            <PrimarySettingsColumn
              snapshot={settings.state.snapshot}
              pendingAction={settings.pendingAction}
              actions={catalogActions}
            />
            <div className="flex min-w-0 flex-col gap-3">
              <SettingsReady
                snapshot={settings.state.snapshot}
                pendingAction={settings.pendingAction}
                onMoveModule={settings.moveHudModule}
                onOpenEditor={settings.openHudEditor}
                onSetAlwaysOnTop={settings.setHudAlwaysOnTop}
                onSetModuleVisibility={settings.setHudModuleVisibility}
                onSetWidth={settings.setHudWidth}
              />
              <SecondarySettingsSections
                snapshot={settings.state.snapshot}
                pendingAction={settings.pendingAction}
                actions={catalogActions}
              />
            </div>
          </div>
        )}
      </div>
    </section>
  );
}

function SettingsReady({
  snapshot,
  pendingAction,
  onMoveModule,
  onOpenEditor,
  onSetAlwaysOnTop,
  onSetModuleVisibility,
  onSetWidth,
}: {
  snapshot: SettingsSnapshot;
  pendingAction: string | null;
  onMoveModule: (
    dragged: HudModuleId,
    target: HudModuleId,
    insertAfter: boolean,
  ) => Promise<void>;
  onOpenEditor: () => Promise<void>;
  onSetAlwaysOnTop: (enabled: boolean) => Promise<void>;
  onSetModuleVisibility: (
    module: HudModuleId,
    visible: boolean,
  ) => Promise<void>;
  onSetWidth: (width: number) => Promise<void>;
}) {
  const windowPending = settingsSectionPending(pendingAction, "hud-window");
  const modulesPending = settingsSectionPending(pendingAction, "hud-modules");
  const editorPending = settingsSectionPending(pendingAction, "hud-editor");

  return (
    <div className="flex flex-col gap-3">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <SlidersHorizontal className="size-4" aria-hidden="true" />
            {t("HUD")}
          </CardTitle>
          <CardAction>
            <Badge variant="outline">
              {tf("{0} px", [String(snapshot.hud.width)])}
            </Badge>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-5">
          <HudWindowSection
            snapshot={snapshot}
            disabled={windowPending}
            onSetAlwaysOnTop={onSetAlwaysOnTop}
            onSetWidth={onSetWidth}
          />
          <Separator />
          <HudModuleOrderSection
            snapshot={snapshot}
            disabled={modulesPending}
            onMoveModule={onMoveModule}
            onSetModuleVisibility={onSetModuleVisibility}
          />
        </CardContent>
        <CardFooter className="justify-between gap-3">
          <div className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
            <EyeOff className="size-4 shrink-0" aria-hidden="true" />
            <span>{t("Hidden until opened")}</span>
          </div>
          <Button disabled={editorPending} onClick={() => void onOpenEditor()}>
            <MonitorUp className="size-4" aria-hidden="true" />
            {t("Open HUD Editor")}
          </Button>
        </CardFooter>
      </Card>
    </div>
  );
}

function HudWindowSection({
  snapshot,
  disabled,
  onSetAlwaysOnTop,
  onSetWidth,
}: {
  snapshot: SettingsSnapshot;
  disabled: boolean;
  onSetAlwaysOnTop: (enabled: boolean) => Promise<void>;
  onSetWidth: (width: number) => Promise<void>;
}) {
  const [width, setWidth] = useState(String(snapshot.hud.width));

  useEffect(() => {
    setWidth(String(snapshot.hud.width));
  }, [snapshot.hud.width]);

  const commitWidth = () => {
    const parsed = Number.parseInt(width, 10);
    const next = Number.isFinite(parsed)
      ? Math.min(snapshot.hudWidthMax, Math.max(snapshot.hudWidthMin, parsed))
      : snapshot.hud.width;
    setWidth(String(next));
    if (next !== snapshot.hud.width) {
      void onSetWidth(next);
    }
  };

  const onWidthKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      commitWidth();
      event.currentTarget.blur();
    }
    if (event.key === "Escape") {
      setWidth(String(snapshot.hud.width));
      event.currentTarget.blur();
    }
  };

  return (
    <section>
      <div>
        <h2 className="flex items-center gap-2 text-sm font-medium">
          <MonitorUp className="size-4" aria-hidden="true" />
          {t("HUD Window")}
        </h2>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
          {t(
            "The overlay is created hidden and opens in editing mode on demand.",
          )}
        </p>
      </div>
      <div className="mt-3 grid divide-y border-y min-[760px]:grid-cols-2 min-[760px]:divide-x min-[760px]:divide-y-0">
        <label className="flex items-center justify-between gap-4 py-2.5 min-[760px]:pr-5">
          <span>
            <span className="flex items-center gap-2 text-sm font-medium">
              <Pin
                className="size-4 text-muted-foreground"
                aria-hidden="true"
              />
              {t("Window always on top")}
            </span>
            <span className="mt-0.5 block text-xs text-muted-foreground">
              {t("Keep the HUD above the game and other windows.")}
            </span>
          </span>
          <Switch
            checked={snapshot.alwaysOnTop}
            disabled={disabled}
            aria-label={t("Window always on top")}
            onCheckedChange={(checked) => void onSetAlwaysOnTop(checked)}
          />
        </label>

        <div className="py-2.5 min-[760px]:pl-5">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <label className="text-sm font-medium" htmlFor="settings-hud-width">
              {t("HUD Width")}
            </label>
            <div className="flex items-center gap-2">
              <input
                id="settings-hud-width"
                className="h-8 w-24 rounded-md border bg-card px-2 text-right text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
                type="number"
                inputMode="numeric"
                min={snapshot.hudWidthMin}
                max={snapshot.hudWidthMax}
                step={4}
                value={width}
                disabled={disabled}
                onBlur={commitWidth}
                onChange={(event) => setWidth(event.target.value)}
                onKeyDown={onWidthKeyDown}
              />
              <span className="text-xs text-muted-foreground">px</span>
            </div>
          </div>
          <input
            className="mt-3 w-full accent-primary"
            type="range"
            min={snapshot.hudWidthMin}
            max={Math.min(snapshot.hudWidthMax, 960)}
            step={4}
            value={Math.min(Number(width) || snapshot.hud.width, 960)}
            disabled={disabled}
            aria-label={t("HUD Width")}
            onChange={(event) => setWidth(event.target.value)}
            onPointerUp={commitWidth}
            onKeyUp={(event) => {
              if (
                event.key === "ArrowLeft" ||
                event.key === "ArrowRight" ||
                event.key === "Home" ||
                event.key === "End"
              ) {
                commitWidth();
              }
            }}
          />
          <div className="mt-1 flex justify-between text-[11px] text-muted-foreground">
            <span>{snapshot.hudWidthMin}px</span>
            <span>{Math.min(snapshot.hudWidthMax, 960)}px</span>
          </div>
        </div>
      </div>
    </section>
  );
}

function HudModuleOrderSection({
  snapshot,
  disabled,
  onMoveModule,
  onSetModuleVisibility,
}: {
  snapshot: SettingsSnapshot;
  disabled: boolean;
  onMoveModule: (
    dragged: HudModuleId,
    target: HudModuleId,
    insertAfter: boolean,
  ) => Promise<void>;
  onSetModuleVisibility: (
    module: HudModuleId,
    visible: boolean,
  ) => Promise<void>;
}) {
  const [dragged, setDragged] = useState<HudModuleId | null>(null);
  const order = snapshot.hud.moduleOrder.filter(isHudModuleId);

  const moveAdjacent = (module: HudModuleId, direction: "up" | "down") => {
    const move = adjacentHudModuleMove(order, module, direction);
    if (move !== null) {
      void onMoveModule(move.dragged, move.target, move.insertAfter);
    }
  };

  const dropModule = (
    event: DragEvent<HTMLDivElement>,
    target: HudModuleId,
  ) => {
    event.preventDefault();
    if (dragged === null || dragged === target) {
      setDragged(null);
      return;
    }
    const insertAfter =
      event.clientY >=
      event.currentTarget.getBoundingClientRect().top +
        event.currentTarget.getBoundingClientRect().height / 2;
    void onMoveModule(dragged, target, insertAfter);
    setDragged(null);
  };

  return (
    <section>
      <div>
        <h2 className="text-sm font-medium">{t("HUD Module Order")}</h2>
        <p className="mt-1 text-xs text-muted-foreground">
          {t(
            "Drag modules to reorder them, or use the arrow buttons for precise keyboard control.",
          )}
        </p>
      </div>
      <div className="mt-3 grid grid-cols-1 overflow-hidden rounded-md border min-[720px]:grid-cols-2">
        {order.map((module, index) => {
          const visible = hudModuleVisible(snapshot.hud, module);
          return (
            <div
              className={cn(
                "flex min-h-10 items-center gap-2 px-2 py-1.5 transition-colors min-[720px]:border-b-0",
                index + 1 < order.length && "border-b",
                index % 2 === 0 && "min-[720px]:border-r",
                index < order.length - (order.length % 2 === 0 ? 2 : 1) &&
                  "min-[720px]:border-b",
                dragged === module && "bg-muted opacity-45",
              )}
              draggable={!disabled}
              key={module}
              onDragEnd={() => setDragged(null)}
              onDragOver={(event) => event.preventDefault()}
              onDragStart={(event) => {
                setDragged(module);
                event.dataTransfer.effectAllowed = "move";
                event.dataTransfer.setData("text/plain", module);
              }}
              onDrop={(event) => dropModule(event, module)}
            >
              <GripVertical
                className="size-4 shrink-0 cursor-grab text-muted-foreground"
                aria-hidden="true"
              />
              <span className="min-w-0 flex-1 truncate text-sm">
                {t(MODULE_LABEL_KEYS[module])}
              </span>
              <Button
                size="icon-xs"
                variant="ghost"
                disabled={disabled || index === 0}
                aria-label={tf("Move {0} up", [t(MODULE_LABEL_KEYS[module])])}
                onClick={() => moveAdjacent(module, "up")}
              >
                <ArrowUp aria-hidden="true" />
              </Button>
              <Button
                size="icon-xs"
                variant="ghost"
                disabled={disabled || index + 1 === order.length}
                aria-label={tf("Move {0} down", [t(MODULE_LABEL_KEYS[module])])}
                onClick={() => moveAdjacent(module, "down")}
              >
                <ArrowDown aria-hidden="true" />
              </Button>
              <Switch
                size="sm"
                checked={visible}
                disabled={disabled}
                aria-label={tf("{0} visibility", [
                  t(MODULE_LABEL_KEYS[module]),
                ])}
                onCheckedChange={(checked) =>
                  void onSetModuleVisibility(module, checked)
                }
              />
            </div>
          );
        })}
      </div>
      <p className="mt-2 text-xs text-muted-foreground">
        {t(
          "The canonical module order is shared by Settings and the HUD editor.",
        )}
      </p>
    </section>
  );
}

function SettingsLoading() {
  return (
    <Card aria-label={t("Loading Settings")}>
      <CardHeader>
        <Skeleton className="h-5 w-36" />
        <Skeleton className="h-4 w-64 max-w-full" />
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <Skeleton className="h-32 w-full" />
        <Skeleton className="h-32 w-full" />
      </CardContent>
    </Card>
  );
}

function SettingsError({
  error,
  onRetry,
}: {
  error: SettingsCommandError;
  onRetry: () => void | Promise<void>;
}) {
  return (
    <Alert variant="destructive">
      <TriangleAlert aria-hidden="true" />
      <AlertTitle>{t("Settings could not be loaded")}</AlertTitle>
      <AlertDescription>
        {tf(error.messageKey, error.messageArguments)}
      </AlertDescription>
      <AlertAction>
        <Button size="sm" variant="outline" onClick={() => void onRetry()}>
          {t("Retry")}
        </Button>
      </AlertAction>
    </Alert>
  );
}

function MutationError({
  error,
  onDismiss,
}: {
  error: SettingsCommandError;
  onDismiss: () => void;
}) {
  return (
    <Alert
      className="fixed top-16 right-5 z-50 w-[min(28rem,calc(100vw-2rem))] bg-background/96 shadow-lg backdrop-blur-sm"
      variant="destructive"
    >
      <TriangleAlert aria-hidden="true" />
      <AlertTitle>{t("Setting was not saved")}</AlertTitle>
      <AlertDescription>
        {tf(error.messageKey, error.messageArguments)}
      </AlertDescription>
      <AlertAction>
        <Button
          aria-label={t("Dismiss")}
          size="icon-xs"
          variant="ghost"
          onClick={onDismiss}
        >
          <X aria-hidden="true" />
        </Button>
      </AlertAction>
    </Alert>
  );
}
