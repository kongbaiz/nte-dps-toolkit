import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { listen } from "@tauri-apps/api/event";
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
import { Button } from "@/components/ui/button";
import { characterAttributeUrl } from "@/lib/character-attribute";
import { characterAvatarUrl } from "@/lib/character-avatar";
import { useDismissibleLayer } from "@/hooks/use-dismissible-layer";
import { t, tf, useTranslationRevision } from "@/lib/i18n";
import {
  applySettingsPresentation,
  useSettingsPresentation,
} from "@/lib/settings-presentation";
import { mainDpsClient } from "@/lib/tauri/main-dps-client";
import type { MainDpsDetailFilter } from "@/lib/tauri/main-dps-detail-contract";
import type {
  MainDpsCharacter,
  MainDpsSnapshot,
} from "@/lib/tauri/main-dps-contract";
import { cn } from "@/lib/utils";

import {
  appearancePanelPosition,
  characterAccent,
  damagePercent,
  formatDuration,
  formatMainMetric,
  mainCaptureStatusTone,
  mainCharacterListState,
  mainDpsContentState,
  roundLabel,
} from "./main-dps-model";
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
  const [appearancePosition, setAppearancePosition] = useState<{
    left: number;
    top: number;
  } | null>(null);
  const appearanceButtonRef = useRef<HTMLButtonElement>(null);
  const appearancePanelRef = useRef<HTMLDivElement>(null);
  const [replayOpen, setReplayOpen] = useState(false);
  const [confirmation, setConfirmation] = useState<MainDpsConfirmation | null>(
    null,
  );
  const [resetUndoToken, setResetUndoToken] = useState<string | null>(null);
  const [onboardingHudPreset, setOnboardingHudPreset] = useState<
    "minimal" | "standard" | "detailed"
  >("standard");
  const [abyssCollapsed, setAbyssCollapsed] = useState(false);
  const [hidden, setHidden] = useState<Set<number>>(() => new Set());
  const [context, setContext] = useState<{
    x: number;
    y: number;
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

  useDismissibleLayer({
    open: appearanceOpen,
    layerRef: appearancePanelRef,
    triggerRef: appearanceButtonRef,
    onDismiss: () => {
      setAppearanceOpen(false);
      setAppearancePosition(null);
    },
  });

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
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setContext(null);
    };
    const close = () => setContext(null);
    window.addEventListener("keydown", closeOnEscape);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("keydown", closeOnEscape);
      window.removeEventListener("resize", close);
    };
  }, [context]);

  useLayoutEffect(() => {
    if (!appearanceOpen) return;

    const updatePosition = () => {
      const anchor = appearanceButtonRef.current?.getBoundingClientRect();
      if (anchor === undefined) return;
      setAppearancePosition(
        appearancePanelPosition(anchor, {
          width: window.innerWidth,
          height: window.innerHeight,
        }),
      );
    };
    updatePosition();
    window.addEventListener("resize", updatePosition);
    return () => {
      window.removeEventListener("resize", updatePosition);
    };
  }, [appearanceOpen]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<string>("main-dps-confirmation-requested", (event) => {
      if (event.payload === "start") setConfirmation({ kind: "start" });
      else if (event.payload === "reset") setConfirmation({ kind: "reset" });
    }).then((next) => {
      unlisten = next;
    });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void mainDpsClient
      .subscribeReplayDrops((path) => {
        if (snapshot === null) return;
        if (snapshot.actions.importReplayRequiresConfirmation) {
          setConfirmation({ kind: "drop", path });
          return;
        }
        void effect("import-drop", async () => {
          const result = await mainDpsClient.importReplayPath(path);
          apply(result.snapshot);
        });
      })
      .then((next) => {
        unlisten = next;
      });
    return () => unlisten?.();
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
    <div
      className="main-dps-root motion-content-ready"
      onClick={() => context !== null && setContext(null)}
    >
      <div className="motion-notice-stack">
        {error !== null && (
          <ActionNotice
            status="error"
            message={tf(error.messageKey, error.messageArguments)}
            onDismiss={() => setError(null)}
          />
        )}
        {resetUndoToken !== null && (
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
        {actionNotice !== null && (
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
        <div className="fixed inset-0 z-[70] grid place-items-center bg-black/45 p-4">
          <section
            aria-modal="true"
            role="dialog"
            aria-labelledby="main-dps-confirm-title"
            className="w-full max-w-md rounded-xl border bg-background p-5 shadow-2xl"
          >
            <h2 id="main-dps-confirm-title" className="text-base font-semibold">
              {t(
                confirmation.kind === "start"
                  ? "Confirm Start"
                  : confirmation.kind === "reset"
                    ? "Confirm Reset"
                    : "Confirm Import",
              )}
            </h2>
            <p className="mt-2 text-sm text-muted-foreground">
              {t(
                confirmation.kind === "start"
                  ? "Starting live capture clears the current stats and re-detects the game connection."
                  : confirmation.kind === "reset"
                    ? "This stops the current task and clears this session's stats, abyss state and detail caches."
                    : "Importing a replay stops the current task and clears existing stats.",
              )}
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <Button variant="outline" onClick={() => setConfirmation(null)}>
                {t("Cancel")}
              </Button>
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
          </section>
        </div>
      )}
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
                variant={snapshot.alwaysOnTop ? "default" : "outline"}
                onClick={() =>
                  void mutate("pin", () =>
                    mainDpsClient.setAlwaysOnTop(!snapshot.alwaysOnTop),
                  )
                }
              >
                {t("Pin")}
              </Button>
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
              <Button
                ref={appearanceButtonRef}
                size="sm"
                variant="outline"
                aria-expanded={appearanceOpen}
                onClick={() => {
                  if (appearanceOpen) {
                    setAppearanceOpen(false);
                    setAppearancePosition(null);
                    return;
                  }
                  const anchor =
                    appearanceButtonRef.current?.getBoundingClientRect();
                  if (anchor !== undefined) {
                    setAppearancePosition(
                      appearancePanelPosition(anchor, {
                        width: window.innerWidth,
                        height: window.innerHeight,
                      }),
                    );
                  }
                  setAppearanceOpen(true);
                }}
              >
                {t("Appearance")}
              </Button>
              {appearanceOpen &&
                appearancePosition !== null &&
                createPortal(
                  <div
                    ref={appearancePanelRef}
                    role="dialog"
                    aria-label={t("Appearance")}
                    className="fixed z-50 flex w-44 flex-col gap-2 rounded-xl border bg-popover p-3 shadow-lg"
                    style={appearancePosition}
                  >
                    <span className="text-xs text-muted-foreground">
                      {t("Theme Preset")}
                    </span>
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
                              setAppearancePosition(null);
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
                  </div>,
                  document.body,
                )}
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

            <section className="main-dps-metrics grid grid-cols-2 gap-1.5 min-[500px]:grid-cols-4">
              <Metric
                value={snapshot.readout.summary.teamDps}
                format={formatMainMetric}
                label={t("Team DPS")}
              />
              <Metric
                value={snapshot.readout.summary.totalDamage}
                format={formatMainMetric}
                label={t("Total Damage")}
              />
              <Metric
                value={snapshot.readout.summary.totalDamageTaken}
                format={formatMainMetric}
                label={t("Total Damage Taken")}
                danger
              />
              <Metric
                value={snapshot.readout.summary.durationSeconds}
                format={formatDuration}
                label={t("Time")}
              />
            </section>

            <section className="main-dps-team-section flex min-h-0 flex-1 flex-col gap-2">
              <div className="flex min-w-0 items-center gap-3">
                <h1 className="text-base font-medium">{t("Team")}</h1>
                <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
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
              {snapshot.readout.damageAttribution.totalDamage > 0 && (
                <DamageAttributionStrip
                  attribution={snapshot.readout.damageAttribution}
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
                            mainDpsClient.openCharacterDetails(row.characterId),
                        )
                      }
                      onContext={(event) => {
                        event.preventDefault();
                        setContext({ x: event.clientX, y: event.clientY, row });
                      }}
                    />
                  ))}
                </div>
              )}
            </section>
          </>
        )}
      </main>

      {context !== null && (
        <div
          className="fixed z-40 min-w-52 rounded-lg border bg-popover p-1 shadow-xl"
          style={{
            left: Math.max(8, Math.min(context.x, window.innerWidth - 216)),
            top: Math.max(8, Math.min(context.y, window.innerHeight - 152)),
          }}
          onClick={(event) => event.stopPropagation()}
        >
          <button
            className="flex w-full items-center gap-2 rounded-md px-3 py-2 text-sm hover:bg-muted"
            onClick={() => {
              void effect(`character-details-${context.row.characterId}`, () =>
                mainDpsClient.openCharacterDetails(context.row.characterId),
              );
              setContext(null);
            }}
          >
            <Eye className="size-4" />
            {t("View combat details")}
          </button>
          <button
            className="flex w-full items-center gap-2 rounded-md px-3 py-2 text-sm hover:bg-muted"
            onClick={() => {
              void navigator.clipboard.writeText(
                [
                  tf("Character: {}", [context.row.name]),
                  tf("DPS: {}", [formatMainMetric(context.row.dps)]),
                  tf("Damage: {}", [formatMainMetric(context.row.damage)]),
                  tf("Share: {}%", [context.row.damageSharePercent.toFixed(1)]),
                  tf("Taken: {}", [formatMainMetric(context.row.damageTaken)]),
                ].join("\n"),
              );
              setContext(null);
            }}
          >
            <Copy className="size-4" />
            {t("Copy values")}
          </button>
          <button
            className="flex w-full items-center gap-2 rounded-md px-3 py-2 text-sm hover:bg-muted"
            onClick={() => {
              setHidden((current) =>
                new Set(current).add(context.row.characterId),
              );
              setContext(null);
            }}
          >
            <EyeOff className="size-4" />
            {t("Hide from ranking")}
          </button>
        </div>
      )}
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
  const avatar = characterAvatarUrl(row.characterId);
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
  onOpen,
}: {
  attribution: MainDpsSnapshot["readout"]["damageAttribution"];
  onOpen(filter: MainDpsOpenDetailFilter): void;
}) {
  const percentage = (damage: number) =>
    `${damagePercent(damage, attribution.totalDamage).toFixed(1)}%`;
  const characterDamage = attribution.separateReactionDamage
    ? attribution.characterDirectDamage
    : attribution.characterDirectDamage + attribution.characterReactionDamage;
  const items = [
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
    ...(attribution.separateReactionDamage
      ? [
          {
            label: t("Reaction Damage"),
            value: percentage(attribution.characterReactionDamage),
            filter: "reactionDamage" as const,
          },
        ]
      : attribution.characterReactionDamage > 0
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
    {
      label: t("Shared mechanics"),
      value: percentage(attribution.sharedDamage),
      filter: "sharedMechanics" as const,
    },
    {
      label: t("Unattributed"),
      value: percentage(attribution.unattributedDamage),
      filter: "unattributed" as const,
    },
  ];

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
    </div>
  );
}
function EmptyCombat({
  gameDetected,
  captureRunning,
  importReplay,
  replayOpen,
  setReplayOpen,
  pending,
  start,
  redetect,
}: {
  gameDetected: boolean;
  captureRunning: boolean;
  importReplay(kind: "json" | "pcapng"): Promise<void>;
  replayOpen: boolean;
  setReplayOpen(value: boolean): void;
  pending: string | null;
  start(): void;
  redetect(): void;
}) {
  const replayButtonRef = useRef<HTMLButtonElement>(null);
  const replayMenuRef = useRef<HTMLDivElement>(null);
  useDismissibleLayer({
    open: replayOpen,
    layerRef: replayMenuRef,
    triggerRef: replayButtonRef,
    onDismiss: () => setReplayOpen(false),
  });

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
          {t("Start HTGame.exe")}
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
            ref={replayButtonRef}
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
          <Button
            size="sm"
            variant="outline"
            disabled={pending !== null}
            aria-expanded={replayOpen}
            aria-haspopup="menu"
            onClick={() => setReplayOpen(!replayOpen)}
          >
            {t("Import Replay")}
          </Button>
          {replayOpen && (
            <div
              ref={replayMenuRef}
              role="menu"
              className="absolute bottom-11 left-0 z-20 flex min-w-40 flex-col gap-1 rounded-lg border bg-popover p-1 shadow-lg"
            >
              <button
                className="rounded-md px-3 py-2 text-left text-sm hover:bg-muted"
                onClick={() => void importReplay("json")}
              >
                {t("JSON replay")}
              </button>
              <button
                className="rounded-md px-3 py-2 text-left text-sm hover:bg-muted"
                onClick={() => void importReplay("pcapng")}
              >
                {t("PCAPNG replay")}
              </button>
            </div>
          )}
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
    <div className="fixed inset-0 z-[80] grid place-items-center bg-black/55 p-4">
      <section
        role="dialog"
        aria-modal="true"
        aria-labelledby="onboarding-title"
        className="w-full max-w-lg rounded-2xl border bg-background p-6 shadow-2xl"
      >
        <div className="mb-5 flex gap-2" aria-label={t("Onboarding progress")}>
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
        <h2 id="onboarding-title" className="text-lg font-semibold">
          {t(titles[step])}
        </h2>
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
                  onboarding.gameDetected
                    ? "The game process is detected."
                    : "Start the game before beginning live capture.",
                )}
              </p>
            </div>
          ) : step === 1 ? (
            <div className="space-y-3">
              <p>
                {tf("{} capture devices are currently available.", [
                  String(onboarding.captureDeviceCount),
                ])}
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
              {(["minimal", "standard", "detailed"] as const).map((preset) => (
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
              ))}
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
      </section>
    </div>
  );
}
