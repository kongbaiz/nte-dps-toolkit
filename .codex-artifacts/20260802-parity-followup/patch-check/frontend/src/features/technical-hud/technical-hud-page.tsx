import {
  useEffect,
  useRef,
  useState,
  type DragEvent,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
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

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { t, tf } from "@/lib/i18n";
import { characterAvatarUrl } from "@/lib/character-avatar";
import { characterAccent } from "@/lib/character-color";
import { useDismissibleLayer } from "@/hooks/use-dismissible-layer";
import {
  showMainDpsFromHud,
  startHudWindowDragging,
} from "@/lib/tauri/hud-window-client";
import { cn } from "@/lib/utils";
import type {
  HudCharacterSnapshot,
  HudModuleId,
  HudSnapshot,
  TechnicalSnapshot,
} from "@/lib/tauri/technical-contract";

import {
  captureAction,
  captureMessage,
  formatHudDuration,
  formatHudNumber,
  hudCharacterName,
  hudDataTone,
  hudModuleConfiguredVisible,
  hudModuleDropInsertAfter,
  hudModuleKeyboardMove,
  hudModuleLabelKey,
  hudModulesInOrder,
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
  const {
    state,
    refresh,
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
    <main className="h-screen overflow-hidden select-none">
      <TechnicalContent
        state={state}
        dragFailed={dragFailed}
        onDragPointerDown={startDragging}
        onRetry={refresh}
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
  onPassthroughChange: (enabled: boolean) => Promise<void>;
  onAlwaysOnTopChange: (enabled: boolean) => Promise<void>;
  onHudModuleVisibilityChange: (
    module: HudModuleId,
    visible: boolean,
  ) => Promise<void>;
  onHudModuleMove: (
    dragged: HudModuleId,
    target: HudModuleId,
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
      <p className="hud-text-halo flex items-center gap-2 text-sm text-white/80">
        <Radio
          className="size-4 animate-pulse text-cyan-200"
          aria-hidden="true"
        />
        {t("Waiting for Rust bridge...")}
      </p>
    );
  }

  if (state.status === "error") {
    return (
      <div className="flex items-center gap-2">
        <ShieldCheck className="size-4 text-amber-200" aria-hidden="true" />
        <p className="hud-text-halo max-w-64 text-xs text-amber-100">
          {tf(state.error.messageKey, state.error.messageArguments)}
        </p>
        <Button
          variant="ghost"
          size="sm"
          className="text-white/80 hover:bg-white/10 hover:text-white"
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
        "flex h-full w-full flex-col gap-1.5 p-1",
        surfaceTone === "blurred" &&
          "overflow-hidden rounded-md bg-background/40",
      )}
    >
      {!snapshot.window.passthrough ? (
        <HudEditorRail
          snapshot={snapshot}
          onDragPointerDown={onDragPointerDown}
          onRefresh={onRetry}
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
        <p role="alert" className="hud-text-halo text-xs text-amber-200">
          {tf(
            snapshot.capture.issue.messageKey,
            snapshot.capture.issue.messageArguments,
          )}
        </p>
      )}

      {dragFailed ? (
        <p role="alert" className="hud-text-halo text-xs text-amber-200">
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
  onRefresh: () => Promise<void>;
  onPassthroughChange: (enabled: boolean) => Promise<void>;
  onAlwaysOnTopChange: (enabled: boolean) => Promise<void>;
  onHudModuleVisibilityChange: (
    module: HudModuleId,
    visible: boolean,
  ) => Promise<void>;
  onHudModuleMove: (
    dragged: HudModuleId,
    target: HudModuleId,
    insertAfter: boolean,
  ) => Promise<void>;
  onHudWidthChange: (width: number) => Promise<void>;
  onCaptureStart: () => Promise<void>;
  onCaptureStop: () => Promise<void>;
}

function HudEditorRail({
  snapshot,
  onDragPointerDown,
  onRefresh,
  onPassthroughChange,
  onAlwaysOnTopChange,
  onHudModuleVisibilityChange,
  onHudModuleMove,
  onHudWidthChange,
  onCaptureStart,
  onCaptureStop,
}: HudEditorRailProps) {
  const status = captureMessage(snapshot.capture);
  return (
    <div className="flex min-w-0 items-center gap-1">
      <div
        className="hud-text-halo flex min-w-0 flex-1 cursor-grab items-center gap-1.5 py-1 text-[11px] font-semibold text-white select-none active:cursor-grabbing"
        onPointerDown={onDragPointerDown}
        title={t("Drag this line to move the window.")}
      >
        <GripHorizontal
          className="size-3.5 shrink-0 text-cyan-200"
          aria-hidden="true"
        />
        <span className="truncate">NTE DPS</span>
        <span className="hidden truncate text-[10px] font-normal text-cyan-100/70 min-[350px]:inline">
          {snapshot.hud.dataState === "preview"
            ? `${t("Preview only")} · `
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
              className="text-white/75 hover:bg-white/10 hover:text-white"
              aria-label={t("Main Window")}
              onClick={() => void showMainDpsFromHud()}
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
              className="text-white/75 hover:bg-white/10 hover:text-white"
              aria-label={t("Refresh technical state")}
              onClick={() => void onRefresh()}
            />
          }
        >
          <RefreshCw data-icon="inline-start" aria-hidden="true" />
        </TooltipTrigger>
        <TooltipContent>{t("Refresh technical state")}</TooltipContent>
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
  onVisibilityChange: (module: HudModuleId, visible: boolean) => Promise<void>;
  onMove: (
    dragged: HudModuleId,
    target: HudModuleId,
    insertAfter: boolean,
  ) => Promise<void>;
  onWidthChange: (width: number) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [pendingModule, setPendingModule] = useState<HudModuleId | null>(null);
  const [movePending, setMovePending] = useState(false);
  const [widthPending, setWidthPending] = useState(false);
  const [draggedModule, setDraggedModule] = useState<HudModuleId | null>(null);
  const draggedModuleRef = useRef<HudModuleId | null>(null);
  const moduleRows = useRef(new Map<HudModuleId, HTMLDivElement>());
  const [dropTarget, setDropTarget] = useState<{
    module: HudModuleId;
    insertAfter: boolean;
  } | null>(null);
  const modules = hudModulesInOrder(hud.config);
  const pending = pendingModule !== null || movePending || widthPending;
  useDismissibleLayer({
    open,
    layerRef: menuRef,
    triggerRef,
    onDismiss: () => setOpen(false),
  });

  const setVisibility = async (module: HudModuleId, visible: boolean) => {
    setPendingModule(module);
    try {
      await onVisibilityChange(module, visible);
    } finally {
      setPendingModule((current) => (current === module ? null : current));
    }
  };

  const moveModule = async (
    dragged: HudModuleId,
    target: HudModuleId,
    insertAfter: boolean,
  ) => {
    if (dragged === target) {
      return;
    }
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

  const resetDrag = () => {
    draggedModuleRef.current = null;
    setDraggedModule(null);
    setDropTarget(null);
  };

  const startPointerDrag = (
    event: PointerEvent<HTMLButtonElement>,
    module: HudModuleId,
  ) => {
    if (pending || event.button !== 0 || !event.isPrimary) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    draggedModuleRef.current = module;
    setDraggedModule(module);
    setDropTarget(null);
  };

  const pointerDropTarget = (
    clientX: number,
    clientY: number,
  ): typeof dropTarget => {
    const dragged = draggedModuleRef.current;
    if (dragged === null) {
      return null;
    }

    for (const module of modules) {
      const row = moduleRows.current.get(module);
      if (module === dragged || row === undefined) {
        continue;
      }
      const rect = row.getBoundingClientRect();
      if (
        clientX >= rect.left &&
        clientX <= rect.right &&
        clientY >= rect.top &&
        clientY <= rect.bottom
      ) {
        return {
          module,
          insertAfter: hudModuleDropInsertAfter(clientY, rect.top, rect.height),
        };
      }
    }
    return null;
  };

  const movePointerDrag = (event: PointerEvent<HTMLButtonElement>) => {
    if (draggedModuleRef.current === null) {
      return;
    }
    event.preventDefault();
    const target = pointerDropTarget(event.clientX, event.clientY);
    setDropTarget((current) =>
      current?.module === target?.module &&
      current?.insertAfter === target?.insertAfter
        ? current
        : target,
    );
  };

  const finishPointerDrag = (event: PointerEvent<HTMLButtonElement>) => {
    const dragged = draggedModuleRef.current;
    if (dragged === null) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const target = pointerDropTarget(event.clientX, event.clientY);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    resetDrag();
    if (target !== null && !pending) {
      void moveModule(dragged, target.module, target.insertAfter);
    }
  };

  const moveWithKeyboard = (
    event: KeyboardEvent<HTMLButtonElement>,
    module: HudModuleId,
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
    const intent = hudModuleKeyboardMove(modules, module, direction);
    if (intent === null) {
      return;
    }
    event.preventDefault();
    void moveModule(module, intent.target, intent.insertAfter);
  };

  return (
    <div className="relative">
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              ref={triggerRef}
              variant="ghost"
              size="icon-xs"
              className={cn(
                "text-white/75 hover:bg-white/10 hover:text-white",
                open && "bg-white/10 text-cyan-100",
              )}
              aria-label={t("HUD modules")}
              aria-expanded={open}
              aria-haspopup="menu"
              onClick={() => setOpen((current) => !current)}
            />
          }
        >
          <Settings2 aria-hidden="true" />
        </TooltipTrigger>
        <TooltipContent>{t("HUD modules")}</TooltipContent>
      </Tooltip>

      {open ? (
        <div
          ref={menuRef}
          role="menu"
          aria-label={t("HUD modules")}
          className="absolute top-full right-0 z-30 mt-1 w-48 rounded-md bg-slate-950/90 p-2 text-white shadow-lg ring-1 ring-white/15"
        >
          <p className="mb-1.5 text-[10px] font-semibold tracking-wide text-cyan-100/80">
            {t("HUD modules")}
          </p>
          <div className="flex flex-col gap-1">
            {modules.map((module) => {
              const visible = hudModuleConfiguredVisible(hud.config, module);
              const label = t(hudModuleLabelKey(module));
              const isDropTarget = dropTarget?.module === module;
              return (
                <div
                  key={module}
                  ref={(row) => {
                    if (row === null) {
                      moduleRows.current.delete(module);
                    } else {
                      moduleRows.current.set(module, row);
                    }
                  }}
                  className={cn(
                    "relative flex h-6 items-center gap-1 rounded px-1 text-[11px] hover:bg-white/5",
                    draggedModule === module && "opacity-45",
                  )}
                >
                  {isDropTarget ? (
                    <span
                      aria-hidden="true"
                      className={cn(
                        "pointer-events-none absolute inset-x-1 z-10 h-0.5 rounded-full bg-cyan-300",
                        dropTarget.insertAfter ? "-bottom-0.5" : "-top-0.5",
                      )}
                    />
                  ) : null}
                  <button
                    type="button"
                    className={cn(
                      "grid size-5 shrink-0 touch-none cursor-grab place-items-center rounded text-white/45 hover:bg-white/10 hover:text-white/85 active:cursor-grabbing disabled:cursor-default disabled:opacity-35",
                      draggedModule === module && "cursor-grabbing",
                    )}
                    disabled={pending}
                    aria-label={`${t(
                      "Drag to reorder; use Up and Down arrow keys to move",
                    )}: ${label}`}
                    title={t(
                      "Drag to reorder; use Up and Down arrow keys to move",
                    )}
                    onPointerDown={(event) => startPointerDrag(event, module)}
                    onPointerMove={movePointerDrag}
                    onPointerUp={finishPointerDrag}
                    onPointerCancel={resetDrag}
                    onLostPointerCapture={() => {
                      if (draggedModuleRef.current !== null) {
                        resetDrag();
                      }
                    }}
                    onKeyDown={(event) => moveWithKeyboard(event, module)}
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
                    disabled={pending}
                    onCheckedChange={(enabled) =>
                      void setVisibility(module, enabled)
                    }
                  />
                </div>
              );
            })}
          </div>
          <HudWidthControl
            width={hud.config.width}
            disabled={pending}
            onWidthChange={setWidth}
          />
        </div>
      ) : null}
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
    <div className="mt-2 flex items-center justify-between gap-2 border-t border-white/10 pt-2 text-[11px]">
      <label htmlFor="hud-width" className="shrink-0 text-cyan-100/80">
        {t("HUD Width")}
      </label>
      <div className="flex items-center gap-1">
        <input
          id="hud-width"
          type="number"
          inputMode="numeric"
          step={4}
          value={draft}
          disabled={disabled}
          className="h-6 w-16 rounded border border-white/15 bg-white/5 px-1.5 text-right font-mono text-[11px] text-white outline-none select-text focus:border-cyan-300/70 disabled:opacity-45"
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
        <span className="text-white/50">px</span>
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
            className="text-white/75 hover:bg-white/10 hover:text-white"
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
          <LoaderCircle className="animate-spin" aria-hidden="true" />
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
    dragged: HudModuleId,
    target: HudModuleId,
    insertAfter: boolean,
  ) => Promise<void>;
}) {
  const hud = snapshot.hud;
  const tone = hudDataTone(hud.dataState);
  const modules = visibleHudModules(hud);
  const [draggedModule, setDraggedModule] = useState<HudModuleId | null>(null);
  const [dropTarget, setDropTarget] = useState<{
    module: HudModuleId;
    insertAfter: boolean;
  } | null>(null);
  const [movePending, setMovePending] = useState(false);

  if (tone === "empty") {
    return (
      <p className="hud-text-halo py-4 text-center text-xs text-white/65">
        {t("Waiting for damage data")}
      </p>
    );
  }

  if (tone === "unknown") {
    return (
      <p className="hud-text-halo py-4 text-center text-xs text-amber-200">
        {tf("Unknown status: {0}", [hud.dataState])}
      </p>
    );
  }

  if (hud.summary === null) {
    return (
      <p className="hud-text-halo py-4 text-center text-xs text-white/65">
        {t("Waiting for damage data")}
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-1">
      {modules.map((module) => {
        const content = renderHudModule(
          hud,
          module,
          snapshot.window.passthrough,
        );
        if (content === null) return null;
        if (snapshot.window.passthrough) {
          return (
            <div className="contents" key={module}>
              {content}
            </div>
          );
        }
        return (
          <HudEditableModule
            key={module}
            module={module}
            modules={modules}
            draggedModule={draggedModule}
            dropTarget={dropTarget}
            movePending={movePending}
            onDraggedModuleChange={setDraggedModule}
            onDropTargetChange={setDropTarget}
            onMove={async (dragged, target, insertAfter) => {
              setMovePending(true);
              try {
                await onModuleMove(dragged, target, insertAfter);
              } finally {
                setMovePending(false);
              }
            }}
          >
            {content}
          </HudEditableModule>
        );
      })}
    </div>
  );
}

function renderHudModule(
  hud: HudSnapshot,
  module: HudModuleId,
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
  draggedModule,
  dropTarget,
  movePending,
  onDraggedModuleChange,
  onDropTargetChange,
  onMove,
  children,
}: {
  module: HudModuleId;
  modules: HudModuleId[];
  draggedModule: HudModuleId | null;
  dropTarget: { module: HudModuleId; insertAfter: boolean } | null;
  movePending: boolean;
  onDraggedModuleChange: (module: HudModuleId | null) => void;
  onDropTargetChange: (
    target: { module: HudModuleId; insertAfter: boolean } | null,
  ) => void;
  onMove: (
    dragged: HudModuleId,
    target: HudModuleId,
    insertAfter: boolean,
  ) => Promise<void>;
  children: ReactNode;
}) {
  const selectedTarget = dropTarget?.module === module ? dropTarget : null;
  const dropIntent = (event: DragEvent<HTMLElement>) => {
    if (draggedModule === null || draggedModule === module || movePending) {
      return null;
    }
    const rect = event.currentTarget.getBoundingClientRect();
    return {
      module,
      insertAfter: hudModuleDropInsertAfter(
        event.clientY,
        rect.top,
        rect.height,
      ),
    };
  };
  const updateDropTarget = (event: DragEvent<HTMLElement>) => {
    const target = dropIntent(event);
    if (target === null) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    onDropTargetChange(target);
  };
  const resetDrag = () => {
    onDraggedModuleChange(null);
    onDropTargetChange(null);
  };
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
      className={cn(
        "relative rounded-sm border border-dashed border-emerald-500/70",
        draggedModule === module && "opacity-45",
      )}
      onDragOver={updateDropTarget}
      onDrop={(event) => {
        event.preventDefault();
        const dragged = draggedModule;
        const target = dropIntent(event);
        resetDrag();
        if (dragged !== null && target !== null && !movePending) {
          void onMove(dragged, target.module, target.insertAfter);
        }
      }}
    >
      {selectedTarget ? (
        <span
          className={cn(
            "pointer-events-none absolute inset-x-0 z-20 h-0.5 bg-emerald-300",
            selectedTarget.insertAfter ? "-bottom-px" : "-top-px",
          )}
          aria-hidden="true"
        />
      ) : null}
      <button
        type="button"
        className="hud-text-halo flex h-5 w-full cursor-grab items-center gap-1 bg-emerald-400/15 px-1.5 text-left text-[10px] font-medium text-emerald-100 active:cursor-grabbing"
        draggable={!movePending}
        aria-label={tf("Drag {} to reorder", [t(hudModuleLabelKey(module))])}
        onDragStart={(event) => {
          event.dataTransfer.effectAllowed = "move";
          event.dataTransfer.setData("text/plain", module);
          onDraggedModuleChange(module);
          onDropTargetChange(null);
        }}
        onDragEnd={resetDrag}
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
    <p className="hud-text-halo text-sm font-semibold tracking-wide text-white">
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
            <p className="hud-text-halo text-[10px] text-white/60">
              {t("Team DPS")}
              {hud.config.showDuration
                ? ` · ${formatHudDuration(summary.durationSeconds)}`
                : ""}
            </p>
            <p className="hud-text-halo truncate font-mono text-[26px] leading-8 font-semibold text-cyan-200">
              {formatHudNumber(summary.teamDps)}
            </p>
          </>
        ) : hud.config.showDuration ? (
          <HudMetric
            label={t("Duration")}
            value={formatHudDuration(summary.durationSeconds)}
          />
        ) : null}
      </div>

      {showRight ? (
        <div className="min-w-0 text-right">
          <p className="hud-text-halo text-[10px] text-white/60">
            {summaryLabel(hud)}
          </p>
          <p className="hud-text-halo truncate font-mono text-sm leading-8 text-white/90">
            {summaryValue(hud)}
          </p>
        </div>
      ) : null}

      {hud.config.showCharacterRows ? (
        <div
          className="absolute inset-x-0 bottom-0 flex h-0.5 overflow-hidden rounded-full bg-white/10"
          aria-hidden="true"
        >
          {hud.characters.map((character, index) => (
            <span
              key={character.characterId}
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

function HudMetric({ label, value }: { label: string; value: string }) {
  return (
    <>
      <p className="hud-text-halo text-[10px] text-white/60">{label}</p>
      <p className="hud-text-halo font-mono text-sm leading-8 text-white/90">
        {value}
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
    <div className="flex h-5 items-center justify-between text-[11px]">
      <p className="hud-text-halo truncate text-white/85">
        {hud.config.showAbyssHalf && hud.status.abyssDetected ? half : ""}
      </p>
      {hud.config.showPassthroughState ? (
        <p className="hud-text-halo text-white/60">
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
        />
      ))}
    </section>
  );
}

function HudCharacterRow({
  character,
  color,
}: {
  character: HudCharacterSnapshot;
  color: string;
}) {
  const projectedName = hudCharacterName(character);
  const name =
    character.previewLabelSuffix === null
      ? projectedName || t("Character")
      : `${t("Character")} ${projectedName}`;
  const avatar =
    character.previewLabelSuffix === null
      ? characterAvatarUrl(character.characterId)
      : null;

  return (
    <div className="grid h-6 min-w-0 grid-cols-[5.5rem_1fr_3.2rem_2.5rem] items-center gap-1.5">
      <div className="flex min-w-0 items-center gap-1.5">
        <span
          className="h-6 w-0.5 shrink-0 rounded-full"
          style={{ backgroundColor: color }}
          aria-hidden="true"
        />
        <span
          className="relative grid size-6 shrink-0 place-items-center overflow-hidden rounded-md border border-white/30 text-[9px] font-semibold text-white"
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
        <span className="hud-text-halo truncate text-xs text-white">
          {name}
        </span>
      </div>

      <div
        className="h-1.5 overflow-hidden rounded-full bg-white/10 ring-1 ring-black/60"
        aria-label={`${name} ${character.damageSharePercent.toFixed(1)}%`}
      >
        <div
          className="h-full rounded-full"
          style={{
            width: `${character.damageSharePercent}%`,
            backgroundColor: color,
          }}
        />
      </div>

      <span className="hud-text-halo text-right font-mono text-xs text-white/90">
        {formatHudNumber(character.dps)}
      </span>
      <span className="hud-text-halo text-right text-[10px]" style={{ color }}>
        {character.damageSharePercent.toFixed(1)}%
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
              "text-white/75 hover:bg-white/10 hover:text-white",
              pressed && "bg-cyan-300/20 text-cyan-100",
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
        {label} · {t(pressed ? "Disable" : "Enable")}
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

function summaryValue(hud: HudSnapshot): string {
  const summary = hud.summary;
  if (summary === null) {
    return "";
  }
  if (hud.config.showTotalDamage && hud.config.showDamageTaken) {
    return `${formatHudNumber(summary.totalDamage)} / ${formatHudNumber(
      summary.totalDamageTaken,
    )}`;
  }
  return formatHudNumber(
    hud.config.showTotalDamage ? summary.totalDamage : summary.totalDamageTaken,
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
