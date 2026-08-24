import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Check,
  ChevronLeft,
  ChevronRight,
  Copy,
  Eye,
  EyeOff,
} from "lucide-react";

import { DesktopTitlebar } from "@/components/nte/desktop-titlebar";
import { ActionNotice } from "@/components/nte/action-notice";
import { AnimatedNumber } from "@/components/nte/animated-number";
import { useWindowMotion } from "@/components/nte/window-motion-context";
import { MaxHpCompressionEffect } from "@/components/nte/max-hp-compression-effect";
import {
  AlertDialog,
  AlertDialogBackdrop,
  AlertDialogClose,
  AlertDialogDescription,
  AlertDialogPopup,
  AlertDialogPortal,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBackdrop,
  DialogPopup,
  DialogPortal,
  DialogTitle,
} from "@/components/ui/dialog";
import { dismissLayerWhenClosed } from "@/components/ui/layer-behavior";
import {
  ContextMenu,
  ContextMenuItem,
  ContextMenuPopup,
  ContextMenuPortal,
  ContextMenuPositioner,
  ContextMenuTrigger,
  Menu,
  MenuItem,
  MenuPopup,
  MenuPortal,
  MenuPositioner,
  MenuTrigger,
} from "@/components/ui/menu";
import {
  Popover,
  PopoverClose,
  PopoverPopup,
  PopoverPortal,
  PopoverPositioner,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover";
import { characterAttributeUrl } from "@/lib/character-attribute";
import { useCharacterAvatar } from "@/hooks/use-character-avatar";
import { cleanupAsyncRegistration } from "@/lib/async-cleanup";
import { t, tf, useTranslationRevision } from "@/lib/i18n";
import {
  applySettingsPresentation,
  useSettingsPresentation,
} from "@/lib/settings-presentation";
import { mainDpsClient } from "@/lib/tauri/main-dps-client";
import type { MainDpsDetailFilter } from "@/lib/tauri/main-dps-detail-contract";
import {
  parseMainDpsCommandError,
  type MainDpsCharacter,
  type MainDpsSnapshot,
} from "@/lib/tauri/main-dps-contract";
import { updatePromptClient } from "@/lib/tauri/update-prompt-client";
import type { UpdatePromptSnapshot } from "@/lib/tauri/update-prompt-contract";
import { subscribeMainDpsConfirmationRequested } from "@/lib/tauri/window-events";
import { cn } from "@/lib/utils";

import {
  characterAccent,
  damagePercent,
  formatDuration,
  formatMainMetric,
  mainCaptureStatusTone,
  mainCharacterListState,
  mainDpsContentState,
  roundLabel,
} from "./main-dps-model";
import { UpdatePromptDialog } from "./update-prompt-dialog";
import { useMainDps } from "./use-main-dps";

type MainDpsOpenDetailFilter = Exclude<MainDpsDetailFilter, "qteType">;
type MainDpsConfirmation =
  | { kind: "start" }
  | { kind: "reset" }
  | { kind: "import"; replayKind: "json" | "pcapng" }
  | { kind: "drop"; path: string };

export function MainDpsPage() {
  useTranslationRevision();
  const presentation = useSettingsPresentation();
  const {
    snapshot,
    error,
    pending,
    actionNotice,
    setActionNotice,
    setError,
    apply,
    mutate,
    effect,
  } = useMainDps();
  const { transitionToWindow } = useWindowMotion();
  const [appearanceOpen, setAppearanceOpen] = useState(false);
  const [replayOpen, setReplayOpen] = useState(false);
  const [confirmation, setConfirmation] = useState<MainDpsConfirmation | null>(
    null,
  );
  const [resetUndoToken, setResetUndoToken] = useState<string | null>(null);
  const [updatePrompt, setUpdatePrompt] = useState<UpdatePromptSnapshot | null>(
    null,
  );
  const [updatePromptOpen, setUpdatePromptOpen] = useState(false);
  const [updatePromptPending, setUpdatePromptPending] = useState(false);
  const [updatePromptError, setUpdatePromptError] = useState<string | null>(
    null,
  );
  const dismissedUpdateKey = useRef<string | null>(null);
  const updatePromptRequest = useRef(0);
  const [onboardingHudPreset, setOnboardingHudPreset] = useState<
    "minimal" | "standard" | "detailed"
  >("standard");
  const [abyssCollapsed, setAbyssCollapsed] = useState(false);
  const [hidden, setHidden] = useState<Set<number>>(() => new Set());
  const [context, setContext] = useState<{
    row: MainDpsCharacter;
  } | null>(null);
  const selectedIndex =
    snapshot?.rounds.findIndex(
      (round) => round.id === snapshot.selectedRoundId,
    ) ?? 0;
  const characters = useMemo(
    () =>
      snapshot?.readout.characters.filter(
        (row) => !hidden.has(row.characterId),
      ) ?? [],
    [hidden, snapshot],
  );
  const characterListState = snapshot
    ? mainCharacterListState(
        snapshot.readout.damageAttribution.totalDamage,
        snapshot.readout.characters.length,
        characters.length,
      )
    : "combat-empty";

  const refreshUpdatePrompt = useCallback(
    async (openWhenAvailable: boolean) => {
      const request = ++updatePromptRequest.current;
      try {
        const next = await updatePromptClient.get();
        if (request !== updatePromptRequest.current) return;
        setUpdatePrompt(next);
        if (next.updates.available.length === 0) {
          setUpdatePromptOpen(false);
          return;
        }
        const key = updatePromptKey(next);
        if (openWhenAvailable && dismissedUpdateKey.current !== key) {
          setUpdatePromptOpen(true);
        }
      } catch (value) {
        if (openWhenAvailable) {
          const commandError = parseMainDpsCommandError(value);
          setUpdatePromptError(
            tf(commandError.messageKey, commandError.messageArguments),
          );
        }
      }
    },
    [],
  );

  useEffect(() => {
    void refreshUpdatePrompt(true);
    return cleanupAsyncRegistration(
      updatePromptClient.subscribeAvailable(() => {
        void refreshUpdatePrompt(true);
      }),
    );
  }, [refreshUpdatePrompt]);

  useEffect(() => {
    void refreshUpdatePrompt(false);
  }, [presentation.language, refreshUpdatePrompt]);

  const updateNow = async () => {
    if (updatePrompt === null || updatePromptPending) return;
    ++updatePromptRequest.current;
    const component =
      updatePrompt.updates.prepared?.component ??
      preferredUpdateComponent(updatePrompt);
    if (component === null) return;
    setUpdatePromptPending(true);
    setUpdatePromptError(null);
    try {
      let next = updatePrompt;
      if (next.updates.prepared === null) {
        next = await updatePromptClient.download(component);
        setUpdatePrompt(next);
      }
      if (next.updates.prepared !== null && next.updates.installEnabled) {
        next = await updatePromptClient.install();
        setUpdatePrompt(next);
        if (next.updates.status === "up-to-date") {
          setUpdatePromptOpen(false);
        }
      }
    } catch (value) {
      const commandError = parseMainDpsCommandError(value);
      setUpdatePromptError(
        tf(commandError.messageKey, commandError.messageArguments),
      );
    } finally {
      setUpdatePromptPending(false);
      void refreshUpdatePrompt(false);
    }
  };

  useEffect(() => {
    if (!appearanceOpen) return;
    const closeAppearanceOnWindowBlur = () => setAppearanceOpen(false);
    window.addEventListener("blur", closeAppearanceOnWindowBlur);
    return () => {
      window.removeEventListener("blur", closeAppearanceOnWindowBlur);
    };
  }, [appearanceOpen]);

  useEffect(() => {
    if (snapshot === null) return;
    applySettingsPresentation({
      ...presentation,
      language: snapshot.appearance.language,
      darkMode: snapshot.appearance.darkMode,
      themePreset: snapshot.appearance.themePreset,
      accent: snapshot.appearance.accent,
      density: snapshot.appearance.density,
      reduceMotion: snapshot.appearance.reduceMotion,
    });
  }, [presentation, snapshot]);

  useEffect(() => {
    if (resetUndoToken === null) return;
    const timer = window.setTimeout(() => setResetUndoToken(null), 5_000);
    return () => window.clearTimeout(timer);
  }, [resetUndoToken]);

  useEffect(() => {
    if (context === null) return;
    const closeContextMenuOnWindowBlur = () => setContext(null);
    window.addEventListener("blur", closeContextMenuOnWindowBlur);
    return () => {
      window.removeEventListener("blur", closeContextMenuOnWindowBlur);
    };
  }, [context]);

  useEffect(() => {
    return cleanupAsyncRegistration(
      subscribeMainDpsConfirmationRequested((payload) => {
        if (payload === "start") setConfirmation({ kind: "start" });
        else setConfirmation({ kind: "reset" });
      }),
    );
  }, []);

  useEffect(() => {
    return cleanupAsyncRegistration(
      mainDpsClient.subscribeReplayDrops((path) => {
        if (snapshot === null) return;
        if (snapshot.actions.importReplayRequiresConfirmation) {
          setConfirmation({ kind: "drop", path });
          return;
        }
        void effect("import-drop", async () => {
          const result = await mainDpsClient.importReplayPath(path);
          apply(result.snapshot);
        });
      }),
    );
  }, [apply, effect, snapshot]);

  useEffect(() => {
    if (snapshot === null) return;
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target;
      const editable =
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement ||
        (target instanceof HTMLElement && target.isContentEditable);
      if (event.repeat || editable || event.altKey || event.shiftKey) return;
      if (event.ctrlKey && event.key.toLocaleLowerCase() === "z") {
        if (resetUndoToken === null) return;
        event.preventDefault();
        const token = resetUndoToken;
        setResetUndoToken(null);
        void mutate("undo-reset", () => mainDpsClient.undoReset(token));
      } else if (event.ctrlKey && event.key.toLocaleLowerCase() === "o") {
        event.preventDefault();
        if (snapshot.actions.importReplayRequiresConfirmation)
          setConfirmation({ kind: "import", replayKind: "pcapng" });
        else
          void effect("import-pcapng", async () => {
            const result = await mainDpsClient.importReplay("pcapng");
            apply(result.snapshot);
          });
      } else if (event.ctrlKey && event.key.toLocaleLowerCase() === "k") {
        event.preventDefault();
        void effect("command-palette", () =>
          mainDpsClient.openConsoleShortcut("palette"),
        );
      } else if (!event.ctrlKey && event.key === "F12") {
        event.preventDefault();
        void effect("packets", () =>
          mainDpsClient.openConsoleShortcut("packets"),
        );
      } else if (
        !event.ctrlKey &&
        event.key === "Tab" &&
        snapshot.readout.abyss.detected
      ) {
        event.preventDefault();
        const next =
          snapshot.readout.abyss.half === "first" ? "second" : "first";
        void mutate("abyss-half", () => mainDpsClient.selectAbyssHalf(next));
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [apply, effect, mutate, resetUndoToken, snapshot]);

  if (snapshot === null) {
    return (
      <div className="main-dps-root motion-route-loading grid place-items-center">
        <div
          className="size-8 animate-spin rounded-full border-2 border-muted border-t-foreground"
          aria-label={t("Loading")}
        />
      </div>
    );
  }

  const captureRunning = snapshot.actions.canStopCapture;
  const startCapture = (replaceCurrent = false) => {
    if (!replaceCurrent && snapshot.actions.startCaptureRequiresConfirmation) {
      setConfirmation({ kind: "start" });
      return;
    }
    void mutate("capture", () => mainDpsClient.startCapture(replaceCurrent));
  };
  const resetSession = async (confirmed = false) => {
    if (!confirmed && snapshot.actions.resetRequiresConfirmation) {
      setConfirmation({ kind: "reset" });
      return;
    }
    await effect("reset", async () => {
      const result = await mainDpsClient.reset(confirmed);
      apply(result.snapshot);
      setResetUndoToken(result.undoToken);
    });
  };
  const importReplay = async (
    kind: "json" | "pcapng",
    replaceCurrent = false,
  ) => {
    setReplayOpen(false);
    if (!replaceCurrent && snapshot.actions.importReplayRequiresConfirmation) {
      setConfirmation({ kind: "import", replayKind: kind });
      return;
    }
    await effect(`import-${kind}`, async () => {
      const result = await mainDpsClient.importReplay(kind, replaceCurrent);
      apply(result.snapshot);
    });
  };
  const confirmAction = () => {
    const action = confirmation;
    setConfirmation(null);
    if (action?.kind === "start") startCapture(true);
    else if (action?.kind === "reset") void resetSession(true);
    else if (action?.kind === "import")
      void importReplay(action.replayKind, true);
    else if (action?.kind === "drop")
      void effect("import-drop", async () => {
        const result = await mainDpsClient.importReplayPath(action.path, true);
        apply(result.snapshot);
      });
  };

  return (
    <div className="main-dps-root motion-content-ready">
      <div className="motion-notice-stack">
        {error !== null && (
          <ActionNotice
            status="error"
            message={tf(error.messageKey, error.messageArguments)}
            onDismiss={() => setError(null)}
          />
        )}
        {!presentation.islandNotifications && resetUndoToken !== null && (
          <ActionNotice
            status="success"
            message={t("Session reset · use Undo within 5 seconds")}
            actionLabel={t("Undo")}
            onAction={() => {
              const token = resetUndoToken;
              setResetUndoToken(null);
              void mutate("undo-reset", () => mainDpsClient.undoReset(token));
            }}
          />
        )}
        {!presentation.islandNotifications &&
          actionNotice !== null &&
          isMainActionNoticeWorthy(actionNotice.action) && (
            <ActionNotice
              key={actionNotice.id}
              status={actionNotice.status}
              message={t(mainActionLabelKey(actionNotice.action))}
              detail={t(
                actionNotice.status === "pending" ? "Working..." : "Completed",
              )}
              onDismiss={
                actionNotice.status === "success"
                  ? () => setActionNotice(null)
                  : undefined
              }
            />
          )}
      </div>
      {confirmation !== null && (
        <AlertDialog
          open
          onOpenChange={(open) =>
            dismissLayerWhenClosed(open, () => setConfirmation(null))
          }
        >
          <AlertDialogPortal>
            <AlertDialogBackdrop className="z-[70] bg-black/45" />
            <AlertDialogPopup className="top-1/2 left-1/2 z-[71] w-[calc(100vw-2rem)] max-w-md -translate-x-1/2 -translate-y-1/2 rounded-xl border bg-background p-5 shadow-2xl">
              <AlertDialogTitle className="text-base">
                {t(
                  confirmation.kind === "start"
                    ? "Confirm Start"
                    : confirmation.kind === "reset"
                      ? "Confirm Reset"
                      : "Confirm Import",
                )}
              </AlertDialogTitle>
              <AlertDialogDescription className="mt-2">
                {t(
                  confirmation.kind === "start"
                    ? "Starting live capture clears the current stats and re-detects the game connection."
                    : confirmation.kind === "reset"
                      ? "This stops the current task and clears this session's stats, abyss state and detail caches."
                      : "Importing a replay stops the current task and clears existing stats.",
                )}
              </AlertDialogDescription>
              <div className="mt-5 flex justify-end gap-2">
                <AlertDialogClose render={<Button variant="outline" />}>
                  {t("Cancel")}
                </AlertDialogClose>
                <Button onClick={confirmAction}>
                  {t(
                    confirmation.kind === "start"
                      ? "Start"
                      : confirmation.kind === "reset"
                        ? "Reset"
                        : "Import",
                  )}
                </Button>
              </div>
            </AlertDialogPopup>
          </AlertDialogPortal>
        </AlertDialog>
      )}
      {updatePromptOpen && updatePrompt !== null ? (
        <UpdatePromptDialog
          error={updatePromptError}
          onLater={() => {
            dismissedUpdateKey.current = updatePromptKey(updatePrompt);
            setUpdatePromptOpen(false);
            setUpdatePromptError(null);
          }}
          onUpdate={() => void updateNow()}
          pending={updatePromptPending}
          updates={updatePrompt.updates}
        />
      ) : null}
      {!snapshot.onboarding.done && (
        <OnboardingOverlay
          onboarding={snapshot.onboarding}
          hudPreset={onboardingHudPreset}
          pending={pending !== null}
          onHudPresetChange={setOnboardingHudPreset}
          onStep={(step) =>
            void mutate("onboarding-step", () =>
              mainDpsClient.setOnboardingStep(step),
            )
          }
          onFinish={() =>
            void mutate("onboarding-finish", () =>
              mainDpsClient.finishOnboarding(onboardingHudPreset),
            )
          }
        />
      )}

      <DesktopTitlebar
        title="NTE DPS TOOL"
        alwaysOnTop={snapshot.alwaysOnTop}
        onAlwaysOnTopChange={(enabled) =>
          mutate("pin", () => mainDpsClient.setAlwaysOnTop(enabled))
        }
        status={
          <span
            className={cn(
              "size-2 rounded-full",
              mainCaptureStatusTone(snapshot.capture.phase) === "active" &&
                "bg-emerald-600",
              mainCaptureStatusTone(snapshot.capture.phase) === "transition" &&
                "bg-amber-500",
              mainCaptureStatusTone(snapshot.capture.phase) === "error" &&
                "bg-destructive",
              mainCaptureStatusTone(snapshot.capture.phase) === "idle" &&
                "bg-muted-foreground/45",
            )}
            role="status"
            aria-label={tf(
              snapshot.capture.messageKey,
              snapshot.capture.messageArguments,
            )}
            title={tf(
              snapshot.capture.messageKey,
              snapshot.capture.messageArguments,
            )}
          />
        }
      />

      {snapshot.selectedRoundId === null &&
      (captureRunning ||
        snapshot.replayRunning ||
        snapshot.hasLiveSessionData) &&
      snapshot.dpsTime.warningMessageKey ? (
        <div
          className="border-b border-amber-500/25 bg-amber-500/10 px-3 py-1.5 text-xs text-amber-700 dark:text-amber-300"
          role="status"
        >
          {t(snapshot.dpsTime.warningMessageKey)}
        </div>
      ) : null}

      <main className="main-dps-content">
        {!abyssCollapsed && (
          <section className="main-dps-toolbar flex flex-wrap items-center justify-between gap-1.5">
            <div className="flex flex-wrap gap-1.5">
              <Button
                size="sm"
                onClick={() =>
                  captureRunning
                    ? void mutate("capture", mainDpsClient.stopCapture)
                    : startCapture()
                }
                disabled={
                  pending !== null ||
                  (!captureRunning && !snapshot.actions.canStartCapture)
                }
              >
                {t(captureRunning ? "Stop" : "Start")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={pending !== null || !snapshot.actions.canReset}
                onClick={() => void resetSession()}
              >
                {t("Reset")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={
                  pending !== null || !snapshot.actions.canStartNewRound
                }
                onClick={() => void mutate("new-round", mainDpsClient.newRound)}
              >
                {t("New Round")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={
                  pending !== null ||
                  (!snapshot.actions.canPause && !snapshot.actions.canResume)
                }
                onClick={() =>
                  void mutate("pause", () =>
                    mainDpsClient.setPaused(!snapshot.processingPaused),
                  )
                }
              >
                {t(snapshot.processingPaused ? "Resume" : "Pause")}
              </Button>
              {snapshot.processingPaused && (
                <span className="self-center text-xs text-muted-foreground">
                  {tf("Paused · {} pending events · {} debug packets", [
                    snapshot.pausedPendingEvents,
                    snapshot.pausedDebugPackets,
                  ])}
                </span>
              )}
              {snapshot.readout.abyss.detected && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setAbyssCollapsed((collapsed) => !collapsed)}
                >
                  {t(abyssCollapsed ? "Expand" : "Collapse")}
                </Button>
              )}
              <Button
                size="sm"
                variant="outline"
                onClick={() =>
                  void effect("hud", () =>
                    transitionToWindow(mainDpsClient.openHud),
                  )
                }
              >
                {t("HUD")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() =>
                  void effect("console", mainDpsClient.openConsole)
                }
              >
                {t("Console")}
              </Button>
            </div>
            <div className="relative flex gap-1.5">
              <Button
                size="sm"
                variant={snapshot.passthrough ? "default" : "outline"}
                onClick={() =>
                  void mutate("passthrough", () =>
                    mainDpsClient.setPassthrough(!snapshot.passthrough),
                  )
                }
              >
                {t(snapshot.passthrough ? "Passthrough on" : "Passthrough")}
              </Button>
              <Popover
                modal="trap-focus"
                open={appearanceOpen}
                onOpenChange={setAppearanceOpen}
              >
                <PopoverTrigger render={<Button size="sm" variant="outline" />}>
                  {t("Appearance")}
                </PopoverTrigger>
                <PopoverPortal>
                  <PopoverPositioner align="end" side="bottom">
                    <PopoverPopup className="flex w-44 flex-col gap-2 p-3">
                      <PopoverTitle className="text-xs font-normal text-muted-foreground">
                        {t("Theme Preset")}
                      </PopoverTitle>
                      <div className="grid grid-cols-2 gap-2">
                        {([false, true] as const).map((dark) => (
                          <Button
                            key={String(dark)}
                            size="sm"
                            variant={
                              snapshot.appearance.darkMode === dark
                                ? "default"
                                : "outline"
                            }
                            onClick={() =>
                              void mutate("appearance", async () => {
                                const next = await mainDpsClient.setAppearance(
                                  dark,
                                  snapshot.appearance.opacity,
                                );
                                applySettingsPresentation({
                                  ...presentation,
                                  darkMode: dark,
                                });
                                setAppearanceOpen(false);
                                return next;
                              })
                            }
                          >
                            {t(dark ? "Dark" : "Light")}
                          </Button>
                        ))}
                      </div>
                      <label className="mt-1 flex flex-col gap-1 text-xs text-muted-foreground">
                        <span className="flex justify-between">
                          <span>{t("Opacity")}</span>
                          <span>
                            {Math.round(snapshot.appearance.opacity * 100)}%
                          </span>
                        </span>
                        <input
                          type="range"
                          min="35"
                          max="100"
                          defaultValue={Math.round(
                            snapshot.appearance.opacity * 100,
                          )}
                          onPointerUp={(event) =>
                            void mutate("appearance", () =>
                              mainDpsClient.setAppearance(
                                snapshot.appearance.darkMode,
                                Number(event.currentTarget.value) / 100,
                              ),
                            )
                          }
                          onKeyUp={(event) => {
                            if (
                              event.key === "ArrowLeft" ||
                              event.key === "ArrowRight"
                            )
                              void mutate("appearance", () =>
                                mainDpsClient.setAppearance(
                                  snapshot.appearance.darkMode,
                                  Number(event.currentTarget.value) / 100,
                                ),
                              );
                          }}
                        />
                      </label>
                      <PopoverClose className="sr-only">
                        {t("Close")}
                      </PopoverClose>
                    </PopoverPopup>
                  </PopoverPositioner>
                </PopoverPortal>
              </Popover>
            </div>
          </section>
        )}

        {mainDpsContentState(snapshot.replayRunning) === "replay-loading" ? (
          <ReplayImportLoading />
        ) : (
          <>
            <section className="grid grid-cols-[auto_auto_minmax(9rem,1fr)_auto_auto] items-center gap-1.5 sm:grid-cols-[auto_auto_minmax(14rem,1fr)_auto_auto]">
              <span className="text-muted-foreground">{t("Combat Round")}</span>
              <Button
                size="icon-sm"
                variant="outline"
                aria-label={t("Previous round")}
                disabled={selectedIndex <= 0}
                onClick={() =>
                  void mutate("round", () =>
                    mainDpsClient.selectRound(
                      snapshot.rounds[selectedIndex - 1]?.id ?? null,
                    ),
                  )
                }
              >
                <ChevronLeft />
              </Button>
              <select
                className="h-7 min-w-0 rounded-lg border bg-background px-2 text-sm"
                value={snapshot.selectedRoundId ?? ""}
                onChange={(event) =>
                  void mutate("round", () =>
                    mainDpsClient.selectRound(event.target.value || null),
                  )
                }
              >
                {snapshot.rounds.map((round) => (
                  <option key={round.id ?? "live"} value={round.id ?? ""}>
                    {roundLabel(round)}
                  </option>
                ))}
              </select>
              <Button
                size="icon-sm"
                variant="outline"
                aria-label={t("Next round")}
                disabled={
                  selectedIndex < 0 ||
                  selectedIndex >= snapshot.rounds.length - 1
                }
                onClick={() =>
                  void mutate("round", () =>
                    mainDpsClient.selectRound(
                      snapshot.rounds[selectedIndex + 1]?.id ?? null,
                    ),
                  )
                }
              >
                <ChevronRight />
              </Button>
              {abyssCollapsed && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setAbyssCollapsed(false)}
                >
                  {t("Expand")}
                </Button>
              )}
            </section>

            {snapshot.readout.abyss.detected && !abyssCollapsed && (
              <section className="flex min-h-7 items-center gap-2">
                <strong className="whitespace-nowrap text-sm font-medium">
                  {snapshot.readout.abyss.floor === null
                    ? t("Abyss")
                    : tf("Abyss Floor {}", [
                        String(snapshot.readout.abyss.floor),
                      ])}
                </strong>
                <span className="h-5 w-px bg-border" aria-hidden="true" />
                {(["first", "second"] as const).map((half) => {
                  const selected = snapshot.readout.abyss.half === half;
                  return (
                    <Button
                      key={half}
                      size="sm"
                      variant={selected ? "default" : "ghost"}
                      onClick={() =>
                        void mutate("abyss-half", () =>
                          mainDpsClient.selectAbyssHalf(half),
                        )
                      }
                    >
                      {t(half === "first" ? "First Half" : "Second Half")}
                    </Button>
                  );
                })}
                {snapshot.readout.abyss.success && (
                  <>
                    <span className="h-5 w-px bg-border" aria-hidden="true" />
                    <span className="text-sm font-medium text-emerald-600">
                      {t("Challenge Cleared")}
                    </span>
                  </>
                )}
              </section>
            )}

            {snapshot.display.metrics.length > 0 && (
              <section className="main-dps-metrics grid grid-cols-[repeat(auto-fit,minmax(7rem,1fr))] gap-1.5">
                {snapshot.display.metrics.includes("team-dps") && (
                  <Metric
                    value={snapshot.readout.summary.teamDps}
                    format={formatMainMetric}
                    label={t("Team DPS")}
                  />
                )}
                {snapshot.display.metrics.includes("total-damage") && (
                  <Metric
                    value={snapshot.readout.summary.totalDamage}
                    format={formatMainMetric}
                    label={t("Total Damage")}
                  />
                )}
                {snapshot.display.metrics.includes("total-damage-taken") && (
                  <Metric
                    value={snapshot.readout.summary.totalDamageTaken}
                    format={formatMainMetric}
                    label={t("Total Damage Taken")}
                    danger
                  />
                )}
                {snapshot.display.metrics.includes("duration") && (
                  <Metric
                    value={snapshot.readout.summary.durationSeconds}
                    format={formatDuration}
                    label={t("Time")}
                  />
                )}
              </section>
            )}

            <section className="main-dps-team-section flex min-h-0 flex-1 flex-col gap-2">
              <div className="flex min-w-0 items-center gap-3">
                <h1 className="text-base font-medium">{t("Team")}</h1>
                <span className="main-dps-team-hint min-w-0 flex-1 truncate text-xs text-muted-foreground">
                  {t(
                    "Click a character to view details; right-click to copy data or hide the character",
                  )}
                </span>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={!snapshot.actions.teamDetailsAvailable}
                  onClick={() =>
                    void effect("team-details", () =>
                      mainDpsClient.openTeamDetails("all"),
                    )
                  }
                >
                  {t("Team Combat Details")}
                </Button>
              </div>
              {(snapshot.readout.damageAttribution.totalDamage > 0 ||
                snapshot.readout.damageAttribution.maxHpReduction > 0) && (
                <DamageAttributionStrip
                  attribution={snapshot.readout.damageAttribution}
                  visible={snapshot.display.attributions}
                  onOpen={(filter) =>
                    void effect(`team-details-${filter}`, () =>
                      mainDpsClient.openTeamDetails(filter),
                    )
                  }
                />
              )}
              {characterListState === "combat-empty" ? (
                <EmptyCombat
                  gameDetected={snapshot.gameDetected}
                  gameDetectionStatus={snapshot.gameDetectionStatus}
                  captureRunning={captureRunning}
                  importReplay={importReplay}
                  replayOpen={replayOpen}
                  setReplayOpen={setReplayOpen}
                  pending={pending}
                  start={() => startCapture()}
                  redetect={() =>
                    void effect("redetect", async () => {
                      apply(await mainDpsClient.getSnapshot());
                    })
                  }
                />
              ) : characterListState === "unattributed" ? (
                <div className="m-auto text-sm text-muted-foreground">
                  {t(
                    "No character-attributed damage under the current accounting mode",
                  )}
                </div>
              ) : characterListState === "hidden" ? (
                <div className="m-auto flex flex-col items-center gap-3 text-muted-foreground">
                  <span>{t("All ranking rows are hidden")}</span>
                  <Button
                    variant="outline"
                    onClick={() => setHidden(new Set())}
                  >
                    {t("Show all characters")}
                  </Button>
                </div>
              ) : (
                <ContextMenu
                  open={context !== null}
                  onOpenChange={(open) =>
                    dismissLayerWhenClosed(open, () => setContext(null))
                  }
                >
                  <ContextMenuTrigger
                    render={
                      <div
                        className="main-dps-team-list"
                        style={
                          {
                            "--main-dps-team-row-count": characters.length,
                          } as React.CSSProperties
                        }
                      >
                        {characters.map((row, index) => (
                          <CharacterRow
                            key={row.characterId}
                            row={row}
                            index={index}
                            onClick={() =>
                              void effect(
                                `character-details-${row.characterId}`,
                                () =>
                                  mainDpsClient.openCharacterDetails(
                                    row.characterId,
                                  ),
                              )
                            }
                            onContext={() => setContext({ row })}
                          />
                        ))}
                      </div>
                    }
                  />
                  {context !== null && (
                    <ContextMenuPortal>
                      <ContextMenuPositioner>
                        <ContextMenuPopup
                          aria-label={t("Character actions")}
                          className="motion-popover min-w-52"
                        >
                          <ContextMenuItem
                            className="gap-2 px-3 py-2"
                            onClick={() =>
                              void effect(
                                `character-details-${context.row.characterId}`,
                                () =>
                                  mainDpsClient.openCharacterDetails(
                                    context.row.characterId,
                                  ),
                              )
                            }
                          >
                            <Eye className="size-4" aria-hidden="true" />
                            {t("View combat details")}
                          </ContextMenuItem>
                          <ContextMenuItem
                            className="gap-2 px-3 py-2"
                            onClick={() =>
                              void navigator.clipboard.writeText(
                                [
                                  tf("Character: {}", [context.row.name]),
                                  tf("DPS: {}", [
                                    formatMainMetric(context.row.dps),
                                  ]),
                                  tf("Damage: {}", [
                                    formatMainMetric(context.row.damage),
                                  ]),
                                  tf("Share: {}%", [
                                    context.row.damageSharePercent.toFixed(1),
                                  ]),
                                  tf("Taken: {}", [
                                    formatMainMetric(context.row.damageTaken),
                                  ]),
                                ].join("\n"),
                              )
                            }
                          >
                            <Copy className="size-4" aria-hidden="true" />
                            {t("Copy values")}
                          </ContextMenuItem>
                          <ContextMenuItem
                            className="gap-2 px-3 py-2"
                            onClick={() =>
                              setHidden((current) =>
                                new Set(current).add(context.row.characterId),
                              )
                            }
                          >
                            <EyeOff className="size-4" aria-hidden="true" />
                            {t("Hide from ranking")}
                          </ContextMenuItem>
                        </ContextMenuPopup>
                      </ContextMenuPositioner>
                    </ContextMenuPortal>
                  )}
                </ContextMenu>
              )}
            </section>
          </>
        )}
      </main>
    </div>
  );
}

function ReplayImportLoading() {
  return (
    <section
      className="motion-route-loading grid min-h-0 flex-1 place-items-center"
      aria-live="polite"
      aria-busy="true"
    >
      <div className="flex w-[min(22rem,90%)] flex-col items-center rounded-xl border bg-card px-6 py-10 text-center shadow-sm">
        <span
          className="size-8 animate-spin rounded-full border-2 border-muted border-t-foreground"
          aria-hidden="true"
        />
        <strong className="mt-4 text-sm font-medium">
          {t("Importing and parsing capture")}
        </strong>
        <span className="mt-1 text-xs text-muted-foreground">
          {t("Results appear after the capture has finished loading")}
        </span>
      </div>
    </section>
  );
}

function mainActionLabelKey(action: string): string {
  if (action === "capture") return "Start / Stop Capture";
  if (action === "reset") return "Reset";
  if (action === "undo-reset") return "Undo";
  if (action === "new-round") return "New Round";
  if (action === "pause") return "Pause";
  if (action === "hud") return "HUD";
  if (action === "console") return "Console";
  if (action === "pin") return "Pin";
  if (action === "passthrough") return "Mouse passthrough";
  if (action === "appearance") return "Appearance";
  if (action === "round") return "Combat Round";
  if (action === "abyss-half") return "Abyss";
  if (action.startsWith("import")) return "Import";
  if (action === "command-palette") return "Command palette";
  if (action === "packets") return "Packets";
  if (action.includes("details")) return "Team Combat Details";
  if (action.startsWith("onboarding")) return "Onboarding";
  return "Refresh";
}

function isMainActionNoticeWorthy(action: string): boolean {
  return (
    action === "capture" ||
    action === "reset" ||
    action === "undo-reset" ||
    action === "new-round" ||
    action === "pause" ||
    action === "pin" ||
    action === "passthrough" ||
    action === "redetect" ||
    action === "onboarding-finish" ||
    action.startsWith("import")
  );
}

function Metric({
  value,
  format,
  label,
  danger = false,
}: {
  value: number;
  format(value: number): string;
  label: string;
  danger?: boolean;
}) {
  return (
    <div className="main-dps-metric rounded-lg border bg-card px-2 py-1.5 text-center">
      <div className={cn("text-xl tabular-nums", danger && "text-destructive")}>
        <AnimatedNumber value={value} format={format} />
      </div>
      <div className="text-xs text-muted-foreground">{label}</div>
    </div>
  );
}
function CharacterRow({
  row,
  index,
  onClick,
  onContext,
}: {
  row: MainDpsCharacter;
  index: number;
  onClick(): void;
  onContext(event: React.MouseEvent): void;
}) {
  const avatar = useCharacterAvatar(row.characterId);
  const attribute = characterAttributeUrl(row.attribute);
  const accent = characterAccent(row.characterId, row.color);
  return (
    <button
      type="button"
      className="main-dps-character-row motion-list-item grid items-center gap-2 overflow-hidden rounded-lg border bg-card text-left transition-colors hover:bg-muted/60"
      style={
        {
          "--main-dps-character-accent": accent,
          "--motion-index": index,
        } as React.CSSProperties
      }
      onClick={onClick}
      onContextMenu={onContext}
    >
      <span
        className="absolute inset-y-0 left-0 w-[3px] rounded-l-lg bg-[var(--main-dps-character-accent)]"
        aria-hidden="true"
      />
      <span className="main-dps-character-attribute grid size-5 place-items-center text-[0.625rem] font-medium text-[var(--main-dps-character-accent)]">
        {attribute === null ? (
          `#${index + 1}`
        ) : (
          <img src={attribute} alt="" className="size-5 object-contain" />
        )}
      </span>
      <span className="main-dps-character-avatar grid size-10 place-items-center overflow-hidden rounded-lg bg-muted">
        {avatar === null ? (
          row.name.slice(0, 1)
        ) : (
          <img src={avatar} alt="" className="size-full object-cover" />
        )}
      </span>
      <span className="min-w-0">
        <strong className="block truncate text-sm">{row.name}</strong>
        <span className="main-dps-character-secondary block truncate text-xs text-muted-foreground">
          {tf("{} hits · {}", [row.hits, formatDuration(row.durationSeconds)])}
        </span>
      </span>
      <span className="min-w-0 text-right">
        <strong className="block whitespace-nowrap text-sm tabular-nums">
          <AnimatedNumber value={row.dps} format={formatMainMetric} /> DPS
        </strong>
        <span className="main-dps-character-secondary block truncate whitespace-nowrap text-xs text-muted-foreground">
          {tf("Damage {} · Share {}% · Taken {}", [
            formatMainMetric(row.damage),
            row.damageSharePercent.toFixed(1),
            formatMainMetric(row.damageTaken),
          ])}
        </span>
      </span>
      <span className="absolute inset-x-2 bottom-0.5 h-0.5 overflow-hidden rounded-full bg-muted">
        <span
          className="motion-bar block h-full rounded-full bg-[var(--main-dps-character-accent)]"
          style={{ width: `${Math.min(100, row.damageSharePercent)}%` }}
        />
      </span>
    </button>
  );
}

function DamageAttributionStrip({
  attribution,
  visible,
  onOpen,
}: {
  attribution: MainDpsSnapshot["readout"]["damageAttribution"];
  visible: MainDpsSnapshot["display"]["attributions"];
  onOpen(filter: MainDpsOpenDetailFilter): void;
}) {
  const percentage = (damage: number) =>
    `${damagePercent(damage, attribution.totalDamage).toFixed(1)}%`;
  const characterDamage = attribution.separateReactionDamage
    ? attribution.characterDirectDamage
    : attribution.characterDirectDamage + attribution.characterReactionDamage;
  const items = [
    ...(visible.includes("character")
      ? [
          {
            label: t(
              attribution.separateReactionDamage
                ? "Character direct"
                : "Character attributed",
            ),
            value: percentage(characterDamage),
            filter: attribution.separateReactionDamage
              ? ("characterDirect" as const)
              : ("characterAttributed" as const),
          },
        ]
      : []),
    ...(visible.includes("reaction") && attribution.separateReactionDamage
      ? [
          {
            label: t("Reaction Damage"),
            value: percentage(attribution.characterReactionDamage),
            filter: "reactionDamage" as const,
          },
        ]
      : visible.includes("reaction") && attribution.characterReactionDamage > 0
        ? [
            {
              label: tf("Includes reaction damage: {}%", [
                damagePercent(
                  attribution.characterReactionDamage,
                  attribution.totalDamage,
                ).toFixed(1),
              ]),
              value: null,
              filter: "reactionDamage" as const,
            },
          ]
        : []),
    ...(visible.includes("shared")
      ? [
          {
            label: t("Shared mechanics"),
            value: percentage(attribution.sharedDamage),
            filter: "sharedMechanics" as const,
          },
        ]
      : []),
    ...(visible.includes("unattributed")
      ? [
          {
            label: t("Unattributed"),
            value: percentage(attribution.unattributedDamage),
            filter: "unattributed" as const,
          },
        ]
      : []),
  ];

  const showMaxHpReduction = visible.includes("max-hp-reduction");
  if (items.length === 0 && !showMaxHpReduction) return null;

  return (
    <div className="main-dps-attribution flex min-w-0 flex-wrap items-center gap-1.5 text-xs">
      <span className="font-medium text-muted-foreground">
        {t("Damage attribution")}
      </span>
      {items.map((item) => (
        <button
          type="button"
          key={item.label}
          className="rounded-md border bg-background px-2 py-1 whitespace-nowrap transition-colors hover:bg-muted"
          onClick={() => onOpen(item.filter)}
        >
          {item.label}
          {item.value !== null && ` ${item.value}`}
        </button>
      ))}
      {showMaxHpReduction && (
        <MaxHpCompressionEffect
          key={attribution.maxHpReduction}
          label={t("Life reduction")}
          value={
            attribution.includeMaxHpReductionInTotalDamage
              ? `${formatMainMetric(attribution.maxHpReduction)} · ${percentage(attribution.maxHpReduction)}`
              : formatMainMetric(attribution.maxHpReduction)
          }
          active={attribution.maxHpReduction > 0}
          className="px-2 py-1"
        />
      )}
    </div>
  );
}
function EmptyCombat({
  gameDetected,
  gameDetectionStatus,
  captureRunning,
  importReplay,
  replayOpen,
  setReplayOpen,
  pending,
  start,
  redetect,
}: {
  gameDetected: boolean;
  gameDetectionStatus: "running" | "notRunning" | "probeFailed";
  captureRunning: boolean;
  importReplay(kind: "json" | "pcapng"): Promise<void>;
  replayOpen: boolean;
  setReplayOpen(value: boolean): void;
  pending: string | null;
  start(): void;
  redetect(): void;
}) {
  useEffect(() => {
    if (!replayOpen) return;
    const closeReplayMenuOnWindowBlur = () => setReplayOpen(false);
    window.addEventListener("blur", closeReplayMenuOnWindowBlur);
    return () => {
      window.removeEventListener("blur", closeReplayMenuOnWindowBlur);
    };
  }, [replayOpen, setReplayOpen]);

  return (
    <div className="m-auto flex w-full max-w-[26rem] flex-col gap-3 rounded-xl border bg-card p-5">
      <h2 className="text-base">{t("Ready for your next combat")}</h2>
      <ol className="space-y-2.5 text-base">
        <li className="flex items-center gap-3">
          <span
            className={cn(
              "grid size-6 place-items-center",
              gameDetected ? "text-emerald-600" : "text-muted-foreground",
            )}
          >
            {gameDetected ? <Check className="size-5" /> : "1"}
          </span>
          {t(
            gameDetectionStatus === "probeFailed"
              ? "Game process detection failed."
              : "Start HTGame.exe",
          )}
        </li>
        <li className="flex items-center gap-3">
          <span
            className={cn(
              "grid size-6 place-items-center",
              captureRunning ? "text-emerald-600" : "text-muted-foreground",
            )}
          >
            {captureRunning ? <Check className="size-5" /> : "2"}
          </span>
          {t("Start live capture")}
        </li>
        <li className="flex items-center gap-3">
          <span className="grid size-6 place-items-center text-muted-foreground">
            3
          </span>
          {t("Enter combat and deal damage")}
        </li>
      </ol>
      <div className="flex gap-2">
        {!gameDetected ? (
          <Button
            size="sm"
            variant="outline"
            disabled={pending !== null}
            onClick={redetect}
          >
            {t("Re-detect")}
          </Button>
        ) : null}
        <Button
          size="sm"
          disabled={captureRunning || pending !== null}
          onClick={start}
        >
          {t("Start Capture")}
        </Button>
        <div className="relative">
          <Menu open={replayOpen} onOpenChange={setReplayOpen}>
            <MenuTrigger
              disabled={pending !== null}
              render={<Button size="sm" variant="outline" />}
            >
              {t("Import Replay")}
            </MenuTrigger>
            <MenuPortal>
              <MenuPositioner align="start" side="top" sideOffset={8}>
                <MenuPopup
                  aria-label={t("Import Replay")}
                  className="motion-popover flex min-w-40 flex-col gap-1"
                >
                  <MenuItem
                    className="px-3 py-2"
                    onClick={() => void importReplay("json")}
                  >
                    {t("JSON replay")}
                  </MenuItem>
                  <MenuItem
                    className="px-3 py-2"
                    onClick={() => void importReplay("pcapng")}
                  >
                    {t("PCAPNG replay")}
                  </MenuItem>
                </MenuPopup>
              </MenuPositioner>
            </MenuPortal>
          </Menu>
        </div>
      </div>
      {captureRunning ? (
        <p className="text-xs text-muted-foreground">
          {t("Waiting for the first damage event...")}
        </p>
      ) : null}
    </div>
  );
}

function updatePromptKey(prompt: UpdatePromptSnapshot): string {
  return prompt.updates.available
    .map(
      (update) => `${update.component}:${update.version}:${update.publishedAt}`,
    )
    .join("|");
}

function preferredUpdateComponent(
  prompt: UpdatePromptSnapshot,
): "app" | "mods-plugin" | null {
  return (
    prompt.updates.available.find((update) => update.component === "app")
      ?.component ??
    prompt.updates.available[0]?.component ??
    null
  );
}

function OnboardingOverlay({
  onboarding,
  hudPreset,
  pending,
  onHudPresetChange,
  onStep,
  onFinish,
}: {
  onboarding: MainDpsSnapshot["onboarding"];
  hudPreset: "minimal" | "standard" | "detailed";
  pending: boolean;
  onHudPresetChange(value: "minimal" | "standard" | "detailed"): void;
  onStep(step: number): void;
  onFinish(): void;
}) {
  const step = Math.min(3, onboarding.step);
  const titles = [
    "Welcome to NTE DPS TOOL",
    "Capture environment",
    "Passthrough recovery hotkey",
    "Choose a HUD preset",
  ] as const;
  return (
    <Dialog open>
      <DialogPortal>
        <DialogBackdrop className="z-[80]" />
        <DialogPopup className="top-1/2 left-1/2 z-[81] w-[calc(100vw-2rem)] max-w-lg -translate-x-1/2 -translate-y-1/2 rounded-2xl border bg-background p-6 shadow-2xl">
          <div
            className="mb-5 flex gap-2"
            aria-label={t("Onboarding progress")}
          >
            {[0, 1, 2, 3].map((index) => (
              <span
                key={index}
                className={cn(
                  "h-1.5 flex-1 rounded-full",
                  index <= step ? "bg-primary" : "bg-muted",
                )}
              />
            ))}
          </div>
          <DialogTitle>{t(titles[step])}</DialogTitle>
          <div className="mt-4 min-h-32 text-sm text-muted-foreground">
            {step === 0 ? (
              <div className="space-y-3">
                <p>
                  {t(
                    "This guide checks capture, recovery controls and the initial HUD layout.",
                  )}
                </p>
                <p>
                  {t(
                    onboarding.gameDetectionStatus === "probeFailed"
                      ? "Game process detection failed."
                      : onboarding.gameDetected
                        ? "The game process is detected."
                        : "Start the game before beginning live capture.",
                  )}
                </p>
              </div>
            ) : step === 1 ? (
              <div className="space-y-3">
                <p>
                  {onboarding.captureDevicesAvailable
                    ? tf("{} capture devices are currently available.", [
                        String(onboarding.captureDeviceCount),
                      ])
                    : t("Capture devices are unavailable.")}
                </p>
                <p>
                  {t(
                    "Automatic adapter selection remains the recommended default; manual selection is available in Settings.",
                  )}
                </p>
              </div>
            ) : step === 2 ? (
              <div className="space-y-3">
                <p>
                  {t(
                    "Use the recovery hotkey whenever the HUD is in mouse-passthrough mode.",
                  )}
                </p>
                <kbd className="inline-flex rounded-md border bg-muted px-3 py-1.5 font-mono text-foreground">
                  {onboarding.passthroughHotkeyLabel}
                </kbd>
                {!onboarding.passthroughHotkeyReady ? (
                  <p className="text-amber-600">
                    {t(
                      "The recovery hotkey is not currently available; keep the Console open before enabling passthrough.",
                    )}
                  </p>
                ) : null}
              </div>
            ) : (
              <div className="grid grid-cols-3 gap-2">
                {(["minimal", "standard", "detailed"] as const).map(
                  (preset) => (
                    <button
                      key={preset}
                      className={cn(
                        "rounded-xl border p-3 text-left",
                        hudPreset === preset && "border-primary bg-primary/10",
                      )}
                      onClick={() => onHudPresetChange(preset)}
                    >
                      <strong className="block text-foreground">
                        {t(
                          preset === "minimal"
                            ? "Minimal"
                            : preset === "standard"
                              ? "Standard"
                              : "Detailed",
                        )}
                      </strong>
                      <span className="mt-1 block text-xs">
                        {t(
                          preset === "minimal"
                            ? "Team DPS and duration"
                            : preset === "standard"
                              ? "Core combat summary"
                              : "All HUD modules",
                        )}
                      </span>
                    </button>
                  ),
                )}
              </div>
            )}
          </div>
          <div className="mt-6 flex items-center justify-between gap-2">
            <Button variant="ghost" disabled={pending} onClick={onFinish}>
              {t("Skip")}
            </Button>
            <div className="flex gap-2">
              <Button
                variant="outline"
                disabled={pending || step === 0}
                onClick={() => onStep(step - 1)}
              >
                {t("Back")}
              </Button>
              <Button
                disabled={pending}
                onClick={step === 3 ? onFinish : () => onStep(step + 1)}
              >
                {t(step === 3 ? "Finish" : "Next")}
              </Button>
            </div>
          </div>
        </DialogPopup>
      </DialogPortal>
    </Dialog>
  );
}
