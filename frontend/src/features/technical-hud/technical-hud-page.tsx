import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
  type CSSProperties,
} from "react";
import {
  GripHorizontal,
  GripVertical,
  AppWindow,
  LoaderCircle,
  MousePointer2,
  Pin,
  Play,
  Radio,
  RefreshCw,
  Settings2,
  ShieldCheck,
  Square,
} from "lucide-react";
import { motion } from "motion/react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/form-controls";
import { ActionNotice } from "@/components/nte/action-notice";
import { AnimatedNumber } from "@/components/nte/animated-number";
import { HudModuleSortableList } from "@/components/nte/hud-module-sortable";
import { useWindowMotion } from "@/components/nte/window-motion-context";
import { Switch } from "@/components/ui/switch";
import {
  Popover,
  PopoverClose,
  PopoverPopup,
  PopoverPortal,
  PopoverPositioner,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useHudModuleSortable } from "@/hooks/use-hud-module-sortable";
import { useSettingsPresentation } from "@/lib/settings-presentation";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { t, tf } from "@/lib/i18n";
import { useCharacterAvatar } from "@/hooks/use-character-avatar";
import { characterAccent } from "@/lib/character-color";
import {
  showMainDpsFromHud,
  startHudWindowDragging,
} from "@/lib/tauri/hud-window-client";
import { cn } from "@/lib/utils";
import type { HudCharacterSnapshot } from "@/lib/tauri/generated/HudCharacterSnapshot";
import type { HudModule } from "@/lib/tauri/generated/HudModule";
import type { HudSnapshot } from "@/lib/tauri/generated/HudSnapshot";
import type { TechnicalSnapshot } from "@/lib/tauri/generated/TechnicalSnapshot";

import {
  captureAction,
  captureMessage,
  formatHudDuration,
  formatHudNumber,
  hudCharacterName,
  hudDataTone,
  hudModuleConfiguredVisible,
  hudModuleKeyboardMove,
  hudModuleLabelKey,
  hudSurfaceTone,
  parseHudWidthDraft,
  type TechnicalPageState,
  visibleHudModules,
} from "./technical-view-model";
import { HudMiniTimeline } from "./hud-mini-timeline";
import { useTechnicalState } from "./use-technical-state";

const HUD_ROLE_COLORS = [
  "var(--hud-role-1)",
  "var(--hud-role-2)",
  "var(--hud-role-3)",
  "var(--hud-role-4)",
] as const;

export function TechnicalHudPage() {
  const presentation = useSettingsPresentation();
  const {
    state,
    actionNotice,
    clearActionNotice,
    refresh,
    reset,
    setPassthrough,
    setAlwaysOnTop,
    setHudModuleVisibility,
    moveHudModule,
    setHudWidth,
    startCapture,
    stopCapture,
  } = useTechnicalState();
  const [dragFailed, setDragFailed] = useState(false);

  const startDragging = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !event.isPrimary) {
      return;
    }

    void startHudWindowDragging()
      .then(() => setDragFailed(false))
      .catch((error: unknown) => {
        console.error("HUD window dragging failed", error);
        setDragFailed(true);
      });
  };

  return (
    <main className="relative h-screen overflow-hidden select-none">
      {!presentation.islandNotifications &&
        actionNotice !== null &&
        isTechnicalActionNoticeWorthy(actionNotice.action) && (
          <div className="pointer-events-none absolute inset-x-1 top-1 z-50">
            <ActionNotice
              key={actionNotice.id}
              compact
              status={actionNotice.status}
              message={t(technicalActionLabelKey(actionNotice.action))}
              detail={t(
                actionNotice.status === "pending" ? "Working..." : "Completed",
              )}
              onDismiss={
                actionNotice.status === "success"
                  ? clearActionNotice
                  : undefined
              }
            />
          </div>
        )}
      <TechnicalContent
        state={state}
        dragFailed={dragFailed}
        onDragPointerDown={startDragging}
        onRetry={refresh}
        onReset={reset}
        onPassthroughChange={setPassthrough}
        onAlwaysOnTopChange={setAlwaysOnTop}
        onHudModuleVisibilityChange={setHudModuleVisibility}
        onHudModuleMove={moveHudModule}
        onHudWidthChange={setHudWidth}
        onCaptureStart={startCapture}
        onCaptureStop={stopCapture}
      />
    </main>
  );
}

interface TechnicalContentProps {
  state: TechnicalPageState;
  dragFailed: boolean;
  onDragPointerDown: (event: PointerEvent<HTMLDivElement>) => void;
  onRetry: () => Promise<void>;
  onReset: () => Promise<void>;
  onPassthroughChange: (enabled: boolean) => Promise<void>;
  onAlwaysOnTopChange: (enabled: boolean) => Promise<void>;
  onHudModuleVisibilityChange: (
    module: HudModule,
    visible: boolean,
  ) => Promise<void>;
  onHudModuleMove: (
    dragged: HudModule,
    target: HudModule,
    insertAfter: boolean,
  ) => Promise<void>;
  onHudWidthChange: (width: number) => Promise<void>;
  onCaptureStart: () => Promise<void>;
  onCaptureStop: () => Promise<void>;
}

function TechnicalContent({
  state,
  dragFailed,
  onDragPointerDown,
  onRetry,
  onReset,
  onPassthroughChange,
  onAlwaysOnTopChange,
  onHudModuleVisibilityChange,
  onHudModuleMove,
  onHudWidthChange,
  onCaptureStart,
  onCaptureStop,
}: TechnicalContentProps) {
  if (state.status === "loading") {
    return (
      <p className="motion-route-loading hud-text-halo flex items-center gap-2 text-sm text-foreground/80">
        <span className="animate-pulse">
          <Radio className="size-4 text-info" aria-hidden="true" />
        </span>
        {t("Waiting for Rust bridge...")}
      </p>
    );
  }

  if (state.status === "error") {
    return (
      <div className="flex items-center gap-2">
        <ShieldCheck className="size-4 text-warning" aria-hidden="true" />
        <p className="hud-text-halo max-w-64 text-xs text-warning">
          {tf(state.error.messageKey, state.error.messageArguments)}
        </p>
        <Button
          variant="ghost"
          size="sm"
          className="text-foreground/80 hover:bg-foreground/10 hover:text-foreground"
          onClick={() => void onRetry()}
        >
          <RefreshCw data-icon="inline-start" aria-hidden="true" />
          {t("Retry")}
        </Button>
      </div>
    );
  }

  const { snapshot } = state;
  const surfaceTone = hudSurfaceTone(snapshot.window.passthrough);
  return (
    <div
      className={cn(
        "motion-content-ready flex h-full w-full flex-col gap-1.5 p-1",
        surfaceTone === "blurred" &&
          "overflow-hidden rounded-md bg-background/40",
      )}
    >
      {!snapshot.window.passthrough ? (
        <HudEditorRail
          snapshot={snapshot}
          onDragPointerDown={onDragPointerDown}
          onReset={onReset}
          onPassthroughChange={onPassthroughChange}
          onAlwaysOnTopChange={onAlwaysOnTopChange}
          onHudModuleVisibilityChange={onHudModuleVisibilityChange}
          onHudModuleMove={onHudModuleMove}
          onHudWidthChange={onHudWidthChange}
          onCaptureStart={onCaptureStart}
          onCaptureStop={onCaptureStop}
        />
      ) : null}

      {snapshot.capture.issue === null ? null : (
        <p role="alert" className="hud-text-halo text-xs text-warning">
          {tf(
            snapshot.capture.issue.messageKey,
            snapshot.capture.issue.messageArguments,
          )}
        </p>
      )}

      {dragFailed ? (
        <p role="alert" className="hud-text-halo text-xs text-warning">
          {t("HUD window operation failed.")}
        </p>
      ) : null}

      <HudProjection snapshot={snapshot} onModuleMove={onHudModuleMove} />
    </div>
  );
}

interface HudEditorRailProps {
  snapshot: TechnicalSnapshot;
  onDragPointerDown: (event: PointerEvent<HTMLDivElement>) => void;
  onReset: () => Promise<void>;
  onPassthroughChange: (enabled: boolean) => Promise<void>;
  onAlwaysOnTopChange: (enabled: boolean) => Promise<void>;
  onHudModuleVisibilityChange: (
    module: HudModule,
    visible: boolean,
  ) => Promise<void>;
  onHudModuleMove: (
    dragged: HudModule,
    target: HudModule,
    insertAfter: boolean,
  ) => Promise<void>;
  onHudWidthChange: (width: number) => Promise<void>;
  onCaptureStart: () => Promise<void>;
  onCaptureStop: () => Promise<void>;
}

function HudEditorRail({
  snapshot,
  onDragPointerDown,
  onReset,
  onPassthroughChange,
  onAlwaysOnTopChange,
  onHudModuleVisibilityChange,
  onHudModuleMove,
  onHudWidthChange,
  onCaptureStart,
  onCaptureStop,
}: HudEditorRailProps) {
  const status = captureMessage(snapshot.capture);
  const { transitionToWindow } = useWindowMotion();
  return (
    <div className="flex min-w-0 items-center gap-1">
      <div
        className="hud-text-halo flex min-w-0 flex-1 cursor-grab items-center gap-1.5 py-1 text-xs font-semibold text-foreground select-none active:cursor-grabbing"
        onPointerDown={onDragPointerDown}
        title={t("Drag this line to move the window.")}
      >
        <GripHorizontal
          className="size-3.5 shrink-0 text-info"
          aria-hidden="true"
        />
        <span className="truncate">NTE DPS</span>
        <span className="hidden truncate text-xs font-normal text-info/70 min-[350px]:inline">
          {snapshot.hud.dataState === "preview"
            ? `${t("Preview only")} / `
            : ""}
          {tf(status.key, status.arguments)}
        </span>
      </div>

      <HudCaptureControl
        phase={snapshot.capture.phase}
        onStart={onCaptureStart}
        onStop={onCaptureStop}
      />

      <HudModuleControls
        hud={snapshot.hud}
        onVisibilityChange={onHudModuleVisibilityChange}
        onMove={onHudModuleMove}
        onWidthChange={onHudWidthChange}
      />

      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant="ghost"
              size="icon-xs"
              className="text-foreground/75 hover:bg-foreground/10 hover:text-foreground"
              aria-label={t("Main Window")}
              onClick={() => void transitionToWindow(showMainDpsFromHud)}
            />
          }
        >
          <AppWindow aria-hidden="true" />
        </TooltipTrigger>
        <TooltipContent>{t("Main Window")}</TooltipContent>
      </Tooltip>

      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant="ghost"
              size="icon-xs"
              className="text-foreground/75 hover:bg-foreground/10 hover:text-foreground"
              aria-label={t("Reset")}
              onClick={() => void onReset()}
            />
          }
        >
          <RefreshCw data-icon="inline-start" aria-hidden="true" />
        </TooltipTrigger>
        <TooltipContent>{t("Reset")}</TooltipContent>
      </Tooltip>

      <HudToggle
        icon={<MousePointer2 aria-hidden="true" />}
        label={t("Mouse passthrough")}
        pressed={snapshot.window.passthrough}
        onPressedChange={onPassthroughChange}
      />
      <HudToggle
        icon={<Pin aria-hidden="true" />}
        label={t("Always on top")}
        pressed={snapshot.window.alwaysOnTop}
        onPressedChange={onAlwaysOnTopChange}
      />
    </div>
  );
}

function HudModuleControls({
  hud,
  onVisibilityChange,
  onMove,
  onWidthChange,
}: {
  hud: HudSnapshot;
  onVisibilityChange: (module: HudModule, visible: boolean) => Promise<void>;
  onMove: (
    dragged: HudModule,
    target: HudModule,
    insertAfter: boolean,
  ) => Promise<void>;
  onWidthChange: (width: number) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [pendingModule, setPendingModule] = useState<HudModule | null>(null);
  const [movePending, setMovePending] = useState(false);
  const [widthPending, setWidthPending] = useState(false);
  const modules = hud.config.moduleOrder;
  const pending = pendingModule !== null || movePending || widthPending;
  useEffect(() => {
    if (!open) return;
    const closeModulesOnWindowBlur = () => setOpen(false);
    window.addEventListener("blur", closeModulesOnWindowBlur);
    return () => {
      window.removeEventListener("blur", closeModulesOnWindowBlur);
    };
  }, [open]);

  const setVisibility = async (module: HudModule, visible: boolean) => {
    setPendingModule(module);
    try {
      await onVisibilityChange(module, visible);
    } finally {
      setPendingModule((current) => (current === module ? null : current));
    }
  };

  const moveModule = async (
    dragged: HudModule,
    target: HudModule,
    insertAfter: boolean,
  ) => {
    setMovePending(true);
    try {
      await onMove(dragged, target, insertAfter);
    } finally {
      setMovePending(false);
    }
  };

  const setWidth = async (width: number) => {
    setWidthPending(true);
    try {
      await onWidthChange(width);
    } finally {
      setWidthPending(false);
    }
  };

  const moveWithKeyboard = (
    event: KeyboardEvent<HTMLButtonElement>,
    order: readonly HudModule[],
    module: HudModule,
  ) => {
    const direction =
      event.key === "ArrowUp"
        ? "up"
        : event.key === "ArrowDown"
          ? "down"
          : null;
    if (direction === null || pending) {
      return;
    }
    const intent = hudModuleKeyboardMove(order, module, direction);
    if (intent === null) {
      return;
    }
    event.preventDefault();
    void moveModule(module, intent.target, intent.insertAfter);
  };

  return (
    <div className="relative">
      <Popover modal="trap-focus" open={open} onOpenChange={setOpen}>
        <Tooltip>
          <TooltipTrigger
            render={
              <PopoverTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    className={cn(
                      "text-foreground/75 hover:bg-foreground/10 hover:text-foreground",
                      open && "bg-foreground/10 text-info",
                    )}
                    aria-label={t("HUD modules")}
                  >
                    <Settings2 aria-hidden="true" />
                  </Button>
                }
              />
            }
          />
          <TooltipContent>{t("HUD modules")}</TooltipContent>
        </Tooltip>
        <PopoverPortal>
          <PopoverPositioner align="end" className="z-30" side="bottom">
            <PopoverPopup
              aria-label={t("HUD modules")}
              className="motion-popover w-48 rounded-md border-0 bg-background/90 p-2 text-foreground shadow-lg ring-1 ring-foreground/15"
            >
              <PopoverTitle className="mb-1.5 text-xs font-semibold tracking-wide text-info/80">
                {t("HUD modules")}
              </PopoverTitle>
              <div className="flex flex-col gap-1">
                <HudModuleSortableList
                  modules={modules}
                  disabled={pending}
                  onMove={moveModule}
                >
                  {(order) =>
                    order.map((module) => (
                      <HudModuleControlRow
                        key={module}
                        module={module}
                        visible={hudModuleConfiguredVisible(hud.config, module)}
                        disabled={pending}
                        onKeyDown={(event) =>
                          moveWithKeyboard(event, order, module)
                        }
                        onVisibilityChange={(visible) =>
                          void setVisibility(module, visible)
                        }
                      />
                    ))
                  }
                </HudModuleSortableList>
              </div>
              <HudWidthControl
                width={hud.config.width}
                disabled={pending}
                onWidthChange={setWidth}
              />
              <PopoverClose className="sr-only">{t("Close")}</PopoverClose>
            </PopoverPopup>
          </PopoverPositioner>
        </PopoverPortal>
      </Popover>
    </div>
  );
}

function HudModuleControlRow({
  module,
  visible,
  disabled,
  onKeyDown,
  onVisibilityChange,
}: {
  module: HudModule;
  visible: boolean;
  disabled: boolean;
  onKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => void;
  onVisibilityChange: (visible: boolean) => void;
}) {
  const sortable = useHudModuleSortable(module);
  const label = t(hudModuleLabelKey(module));
  return (
    <div
      ref={sortable.rowRef}
      style={sortable.rowStyle}
      className={cn(
        "relative flex h-6 items-center gap-1 rounded px-1 text-xs hover:bg-foreground/5",
        sortable.dragging && "z-10 bg-foreground/10",
      )}
    >
      <button
        ref={sortable.handleRef}
        type="button"
        className={cn(
          "grid size-5 shrink-0 touch-none cursor-grab place-items-center rounded text-foreground/45 hover:bg-foreground/10 hover:text-foreground/85 active:cursor-grabbing disabled:cursor-default disabled:opacity-35",
          sortable.dragging && "cursor-grabbing",
        )}
        disabled={disabled}
        aria-label={`${t(
          "Drag to reorder; use Up and Down arrow keys to move",
        )}: ${label}`}
        title={t("Drag to reorder; use Up and Down arrow keys to move")}
        {...sortable.handleListeners}
        onKeyDown={onKeyDown}
      >
        <GripVertical className="size-3.5" aria-hidden="true" />
      </button>
      <label
        htmlFor={`hud-module-${module}`}
        className="min-w-0 flex-1 truncate"
      >
        {label}
      </label>
      <Switch
        id={`hud-module-${module}`}
        aria-label={t(visible ? "Hide module" : "Restore module")}
        checked={visible}
        disabled={disabled}
        onCheckedChange={onVisibilityChange}
      />
    </div>
  );
}

function HudWidthControl({
  width,
  disabled,
  onWidthChange,
}: {
  width: number;
  disabled: boolean;
  onWidthChange: (width: number) => Promise<void>;
}) {
  const [draft, setDraft] = useState(String(width));
  const committing = useRef(false);

  useEffect(() => {
    setDraft(String(width));
  }, [width]);

  const commit = async () => {
    if (disabled || committing.current) {
      return;
    }
    const parsed = parseHudWidthDraft(draft);
    if (parsed === null || parsed === width) {
      setDraft(String(width));
      return;
    }
    committing.current = true;
    try {
      await onWidthChange(parsed);
    } finally {
      committing.current = false;
    }
  };

  return (
    <div className="mt-2 flex items-center justify-between gap-2 border-t border-foreground/10 pt-2 text-xs">
      <label htmlFor="hud-width" className="shrink-0 text-info/80">
        {t("HUD Width")}
      </label>
      <div className="flex items-center gap-1">
        <Input
          id="hud-width"
          type="number"
          inputMode="numeric"
          step={4}
          value={draft}
          disabled={disabled}
          className="h-6 w-16 border-foreground/15 bg-foreground/5 px-1.5 text-right font-mono text-xs text-foreground select-text"
          aria-label={t("HUD Width")}
          onChange={(event) => setDraft(event.currentTarget.value)}
          onFocus={(event) => event.currentTarget.select()}
          onBlur={() => void commit()}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void commit();
            } else if (event.key === "Escape") {
              event.preventDefault();
              setDraft(String(width));
            }
          }}
        />
        <span className="text-foreground/50">px</span>
      </div>
    </div>
  );
}

function HudCaptureControl({
  phase,
  onStart,
  onStop,
}: {
  phase: string;
  onStart: () => Promise<void>;
  onStop: () => Promise<void>;
}) {
  const action = captureAction(phase);
  const label =
    action === "start"
      ? t("Start Capture")
      : action === "stop"
        ? t("Stop Capture")
        : t(
            phase === "stopping"
              ? "Stopping live capture..."
              : "Starting live capture...",
          );

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon-xs"
            className="text-foreground/75 hover:bg-foreground/10 hover:text-foreground"
            aria-label={label}
            disabled={action === "pending"}
            onClick={() => void (action === "start" ? onStart() : onStop())}
          />
        }
      >
        {action === "start" ? (
          <Play aria-hidden="true" />
        ) : action === "stop" ? (
          <Square aria-hidden="true" />
        ) : (
          <span className="animate-spin">
            <LoaderCircle aria-hidden="true" />
          </span>
        )}
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

function HudProjection({
  snapshot,
  onModuleMove,
}: {
  snapshot: TechnicalSnapshot;
  onModuleMove: (
    dragged: HudModule,
    target: HudModule,
    insertAfter: boolean,
  ) => Promise<void>;
}) {
  const hud = snapshot.hud;
  const tone = hudDataTone(hud.dataState);
  const modules = visibleHudModules(hud);
  const moduleLayoutKey = modules.join("|");
  const [movePending, setMovePending] = useState(false);
  const moveModule = async (
    dragged: HudModule,
    target: HudModule,
    insertAfter: boolean,
  ) => {
    setMovePending(true);
    try {
      await onModuleMove(dragged, target, insertAfter);
    } finally {
      setMovePending(false);
    }
  };

  if (tone === "empty") {
    return (
      <p className="hud-text-halo py-4 text-center text-xs text-foreground/65">
        {t("Waiting for damage data")}
      </p>
    );
  }

  if (tone === "unknown") {
    return (
      <p className="hud-text-halo py-4 text-center text-xs text-warning">
        {tf("Unknown status: {0}", [hud.dataState])}
      </p>
    );
  }

  if (hud.summary === null) {
    return (
      <p className="hud-text-halo py-4 text-center text-xs text-foreground/65">
        {t("Waiting for damage data")}
      </p>
    );
  }

  const passthrough = snapshot.window.passthrough;
  const contents = new Map<HudModule, ReactNode>();
  for (const module of modules) {
    const content = renderHudModule(hud, module, passthrough);
    if (content !== null) contents.set(module, content);
  }
  return (
    <div className="flex flex-col gap-1">
      <HudModuleSortableList
        modules={[...contents.keys()]}
        disabled={movePending || passthrough}
        onMove={moveModule}
      >
        {(order) =>
          order.map((module) => (
            <HudLayoutModule
              key={module}
              module={module}
              layoutKey={moduleLayoutKey}
            >
              {passthrough ? (
                contents.get(module)
              ) : (
                <HudEditableModule
                  module={module}
                  modules={order}
                  movePending={movePending}
                  onMove={moveModule}
                >
                  {contents.get(module)}
                </HudEditableModule>
              )}
            </HudLayoutModule>
          ))
        }
      </HudModuleSortableList>
    </div>
  );
}

function HudLayoutModule({
  module,
  layoutKey,
  children,
}: {
  module: HudModule;
  layoutKey: string;
  children: ReactNode;
}) {
  // Rust's order is the dependency: a dropped drag is already in place, so only
  // reorders that arrive from elsewhere (popover, keyboard, Settings) animate.
  return (
    <motion.div
      layout="position"
      layoutDependency={layoutKey}
      className="hud-layout-module"
      data-hud-module={module}
    >
      {children}
    </motion.div>
  );
}

function renderHudModule(
  hud: HudSnapshot,
  module: HudModule,
  passthrough: boolean,
): ReactNode {
  switch (module) {
    case "title":
      return <HudTitle />;
    case "summary":
      return <HudSummary hud={hud} />;
    case "status":
      return <HudStatus hud={hud} passthrough={passthrough} />;
    case "characters":
      return <HudCharacters hud={hud} />;
    case "timeline":
      return hud.timeline === null ? null : (
        <HudMiniTimeline timeline={hud.timeline} interactive={!passthrough} />
      );
    default:
      return null;
  }
}

function HudEditableModule({
  module,
  modules,
  movePending,
  onMove,
  children,
}: {
  module: HudModule;
  modules: readonly HudModule[];
  movePending: boolean;
  onMove: (
    dragged: HudModule,
    target: HudModule,
    insertAfter: boolean,
  ) => Promise<void>;
  children: ReactNode;
}) {
  const sortable = useHudModuleSortable(module);
  const moveWithKeyboard = (event: KeyboardEvent<HTMLButtonElement>) => {
    const direction =
      event.key === "ArrowUp"
        ? "up"
        : event.key === "ArrowDown"
          ? "down"
          : null;
    if (direction === null || movePending) return;
    const intent = hudModuleKeyboardMove(modules, module, direction);
    if (intent === null) return;
    event.preventDefault();
    void onMove(module, intent.target, intent.insertAfter);
  };

  return (
    <section
      ref={sortable.rowRef}
      style={sortable.rowStyle}
      className={cn(
        "relative rounded-sm border border-dashed border-success/70",
        sortable.dragging && "z-10",
      )}
    >
      <button
        ref={sortable.handleRef}
        type="button"
        className={cn(
          "hud-text-halo flex h-5 w-full touch-none cursor-grab select-none items-center gap-1 bg-success/15 px-1.5 text-left text-xs font-medium text-success active:cursor-grabbing",
          sortable.dragging && "cursor-grabbing",
        )}
        disabled={movePending}
        aria-label={tf("Drag {} to reorder", [t(hudModuleLabelKey(module))])}
        {...sortable.handleListeners}
        onKeyDown={moveWithKeyboard}
      >
        <GripVertical className="size-3" aria-hidden="true" />
        <span>{t(hudModuleLabelKey(module))}</span>
      </button>
      <div className="px-1 py-0.5">{children}</div>
    </section>
  );
}

function HudTitle() {
  return (
    <p className="hud-text-halo text-sm font-semibold tracking-wide text-foreground">
      NTE DPS
    </p>
  );
}

function HudSummary({ hud }: { hud: HudSnapshot }) {
  const summary = hud.summary;
  if (summary === null) {
    return null;
  }

  const showRight = hud.config.showTotalDamage || hud.config.showDamageTaken;
  return (
    <section
      className="relative grid min-h-14 grid-cols-2 items-start"
      aria-label={t("Summary")}
    >
      <div className="min-w-0">
        {hud.config.showTeamDps ? (
          <>
            <p className="hud-text-halo text-xs text-foreground/60">
              {t("Team DPS")}
              {hud.config.showDuration ? (
                <>
                  {" / "}
                  <AnimatedNumber
                    value={summary.durationSeconds}
                    format={formatHudDuration}
                  />
                </>
              ) : null}
            </p>
            <p className="hud-text-halo truncate font-mono text-2xl leading-8 font-semibold text-info">
              <AnimatedNumber
                value={summary.teamDps}
                format={formatHudNumber}
              />
            </p>
          </>
        ) : hud.config.showDuration ? (
          <HudMetric
            label={t("Duration")}
            value={summary.durationSeconds}
            format={formatHudDuration}
          />
        ) : null}
      </div>

      {showRight ? (
        <div className="min-w-0 text-right">
          <p className="hud-text-halo text-xs text-foreground/60">
            {summaryLabel(hud)}
          </p>
          <p className="hud-text-halo truncate font-mono text-sm leading-8 text-foreground/90">
            <HudSummaryValue hud={hud} />
          </p>
        </div>
      ) : null}

      {hud.config.showCharacterRows ? (
        <div
          className="absolute inset-x-0 bottom-0 flex h-0.5 overflow-hidden rounded-full bg-foreground/10"
          aria-hidden="true"
        >
          {hud.characters.map((character, index) => (
            <span
              key={character.characterId}
              className="motion-bar"
              style={{
                width: `${character.damageSharePercent}%`,
                backgroundColor: hudCharacterColor(character, index),
              }}
            />
          ))}
        </div>
      ) : null}
    </section>
  );
}

function HudMetric({
  label,
  value,
  format,
}: {
  label: string;
  value: number;
  format(value: number): string;
}) {
  return (
    <>
      <p className="hud-text-halo text-xs text-foreground/60">{label}</p>
      <p className="hud-text-halo font-mono text-sm leading-8 text-foreground/90">
        <AnimatedNumber value={value} format={format} />
      </p>
    </>
  );
}

function HudStatus({
  hud,
  passthrough,
}: {
  hud: HudSnapshot;
  passthrough: boolean;
}) {
  const half =
    hud.status.abyssHalf === "first"
      ? t("Ascending Line")
      : hud.status.abyssHalf === "second"
        ? t("Descending Line")
        : null;

  return (
    <div className="flex h-5 items-center justify-between text-xs">
      <p className="hud-text-halo truncate text-foreground/85">
        {hud.config.showAbyssHalf && hud.status.abyssDetected ? half : ""}
      </p>
      {hud.config.showPassthroughState ? (
        <p className="hud-text-halo text-foreground/60">
          {t(passthrough ? "Passthrough" : "Edit")}
        </p>
      ) : null}
    </div>
  );
}

function HudCharacters({ hud }: { hud: HudSnapshot }) {
  return (
    <section
      className="flex flex-col gap-1"
      aria-label={t("Character Ranking")}
    >
      {hud.characters.map((character, index) => (
        <HudCharacterRow
          key={character.characterId}
          character={character}
          color={hudCharacterColor(character, index)}
          index={index}
        />
      ))}
    </section>
  );
}

function HudCharacterRow({
  character,
  color,
  index,
}: {
  character: HudCharacterSnapshot;
  color: string;
  index: number;
}) {
  const projectedName = hudCharacterName(character);
  const name =
    character.previewLabelSuffix === null
      ? projectedName || t("Character")
      : `${t("Character")} ${projectedName}`;
  const catalogAvatar = useCharacterAvatar(character.characterId);
  const avatar = character.previewLabelSuffix === null ? catalogAvatar : null;

  return (
    <div
      className="motion-list-item grid h-6 min-w-0 grid-cols-[5.5rem_1fr_3.2rem_2.5rem] items-center gap-1.5"
      style={{ "--motion-index": index } as CSSProperties}
    >
      <div className="flex min-w-0 items-center gap-1.5">
        <span
          className="h-6 w-0.5 shrink-0 rounded-full"
          style={{ backgroundColor: color }}
          aria-hidden="true"
        />
        <span
          className="relative grid size-6 shrink-0 place-items-center overflow-hidden rounded-md border border-foreground/30 text-xs font-semibold text-foreground"
          style={{ backgroundColor: color }}
          title={name}
        >
          <span aria-hidden="true">{name.slice(0, 1)}</span>
          {avatar ? (
            <img
              className="absolute inset-0 size-full object-cover"
              src={avatar}
              alt=""
              draggable={false}
              onError={(event) => event.currentTarget.remove()}
            />
          ) : null}
        </span>
        <span className="hud-text-halo truncate text-xs text-foreground">
          {name}
        </span>
      </div>

      <div
        className="h-1.5 overflow-hidden rounded-full bg-foreground/10 ring-1 ring-black/60"
        aria-label={`${name} ${character.damageSharePercent.toFixed(1)}%`}
      >
        <div
          className="motion-bar h-full rounded-full"
          style={{
            width: `${character.damageSharePercent}%`,
            backgroundColor: color,
          }}
        />
      </div>

      <span className="hud-text-halo text-right font-mono text-xs text-foreground/90">
        <AnimatedNumber value={character.dps} format={formatHudNumber} />
      </span>
      <span className="hud-text-halo text-right text-xs" style={{ color }}>
        <AnimatedNumber
          value={character.damageSharePercent}
          format={(value) => `${value.toFixed(1)}%`}
        />
      </span>
    </div>
  );
}

interface HudToggleProps {
  icon: ReactNode;
  label: string;
  pressed: boolean;
  onPressedChange: (enabled: boolean) => Promise<void>;
}

function HudToggle({ icon, label, pressed, onPressedChange }: HudToggleProps) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon-xs"
            className={cn(
              "text-foreground/75 hover:bg-foreground/10 hover:text-foreground",
              pressed && "bg-info/20 text-info",
            )}
            aria-label={label}
            aria-pressed={pressed}
            onClick={() => void onPressedChange(!pressed)}
          />
        }
      >
        {icon}
      </TooltipTrigger>
      <TooltipContent>
        {label} / {t(pressed ? "Disable" : "Enable")}
      </TooltipContent>
    </Tooltip>
  );
}

function summaryLabel(hud: HudSnapshot): string {
  if (hud.config.showTotalDamage && hud.config.showDamageTaken) {
    return t("Total Damage / Taken");
  }
  return t(hud.config.showTotalDamage ? "Total Damage" : "Total Damage Taken");
}

function HudSummaryValue({ hud }: { hud: HudSnapshot }) {
  const summary = hud.summary;
  if (summary === null) {
    return null;
  }
  if (hud.config.showTotalDamage && hud.config.showDamageTaken) {
    return (
      <>
        <AnimatedNumber value={summary.totalDamage} format={formatHudNumber} />
        {" / "}
        <AnimatedNumber
          value={summary.totalDamageTaken}
          format={formatHudNumber}
        />
      </>
    );
  }
  return (
    <AnimatedNumber
      value={
        hud.config.showTotalDamage
          ? summary.totalDamage
          : summary.totalDamageTaken
      }
      format={formatHudNumber}
    />
  );
}

function technicalActionLabelKey(action: string): string {
  if (action === "refresh") return "Refresh technical state";
  if (action === "reset") return "Reset";
  if (action === "passthrough") return "Mouse passthrough";
  if (action === "always-on-top") return "Always on top";
  if (action === "hud-width") return "HUD Width";
  if (action === "start-capture") return "Start Capture";
  if (action === "stop-capture") return "Stop Capture";
  return "HUD modules";
}

function isTechnicalActionNoticeWorthy(action: string): boolean {
  return (
    action === "passthrough" ||
    action === "always-on-top" ||
    action === "start-capture" ||
    action === "stop-capture"
  );
}

function hudRoleColor(index: number): string {
  return HUD_ROLE_COLORS[index % HUD_ROLE_COLORS.length];
}

function hudCharacterColor(
  character: HudCharacterSnapshot,
  index: number,
): string {
  return character.previewLabelSuffix === null
    ? characterAccent(character.characterId, character.color)
    : hudRoleColor(index);
}
