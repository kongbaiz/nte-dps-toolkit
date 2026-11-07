import { useEffect, useEffectEvent, useState } from "react";
import { ChevronLeft, ChevronRight, LoaderCircle } from "lucide-react";

import { DesktopTitlebar } from "@/components/nte/desktop-titlebar";
import { ActionNotice } from "@/components/nte/action-notice";
import { AnimatedNumber } from "@/components/nte/animated-number";
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
import { NativeSelect } from "@/components/ui/form-controls";
import { cleanupAsyncRegistration } from "@/lib/async-cleanup";
import { t, tf, useFrontendLanguage } from "@/lib/i18n";
import {
  applySettingsPresentation,
  useSettingsPresentation,
} from "@/lib/settings-presentation";
import { mainDpsClient } from "@/lib/tauri/main-dps-client";
import type { MainDpsSnapshot } from "@/lib/tauri/generated/MainDpsSnapshot";
import { subscribeMainDpsConfirmationRequested } from "@/lib/tauri/window-events";
import { cn } from "@/lib/utils";

import {
  formatDuration,
  formatMainMetric,
  mainCaptureStatusTone,
  mainDpsContentState,
  roundLabel,
} from "./main-dps-model";
import { OnboardingOverlay } from "./main-dps-onboarding";
import { MainDpsTeamSection } from "./main-dps-team";
import { MainDpsToolbar } from "./main-dps-toolbar";
import { MainUpdatePrompt } from "./update-prompt-dialog";
import { DataSourceControl } from "./data-source-control";
import { useMainDps } from "./use-main-dps";

type MainDpsControls = ReturnType<typeof useMainDps>;
type MainDpsConfirmation =
  | { kind: "start" }
  | { kind: "reset" }
  | { kind: "import"; replayKind: "json" | "pcapng" }
  | { kind: "drop"; path: string };

export function MainDpsPage() {
  useFrontendLanguage();
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
  const [confirmation, setConfirmation] = useState<MainDpsConfirmation | null>(
    null,
  );
  const [resetUndoToken, setResetUndoToken] = useState<string | null>(null);
  const [abyssCollapsed, setAbyssCollapsed] = useState(false);
  const [hiddenCharacters, setHiddenCharacters] = useState<Set<number>>(
    () => new Set(),
  );

  const { language, darkMode, themePreset, accent, density, reduceMotion } =
    snapshot?.appearance ?? presentation;
  useEffect(() => {
    applySettingsPresentation({
      ...presentation,
      language,
      darkMode,
      themePreset,
      accent,
      density,
      reduceMotion,
    });
  }, [
    presentation,
    language,
    darkMode,
    themePreset,
    accent,
    density,
    reduceMotion,
  ]);

  useEffect(() => {
    if (resetUndoToken === null) return;
    const timer = window.setTimeout(() => setResetUndoToken(null), 5_000);
    return () => window.clearTimeout(timer);
  }, [resetUndoToken]);

  useEffect(() => {
    return cleanupAsyncRegistration(
      subscribeMainDpsConfirmationRequested((payload) => {
        if (payload === "start") setConfirmation({ kind: "start" });
        else setConfirmation({ kind: "reset" });
      }),
    );
  }, []);

  const importDroppedReplay = useEffectEvent((path: string) => {
    if (snapshot === null) return;
    if (snapshot.actions.importReplayRequiresConfirmation) {
      setConfirmation({ kind: "drop", path });
      return;
    }
    void effect("import-drop", async () => {
      const result = await mainDpsClient.importReplayPath(path);
      apply(result.snapshot);
    });
  });

  useEffect(() => {
    return cleanupAsyncRegistration(
      mainDpsClient.subscribeReplayDrops((path) => importDroppedReplay(path)),
    );
  }, []);

  const handleShortcut = useEffectEvent((event: KeyboardEvent) => {
    if (snapshot === null) return;
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
      const next = snapshot.readout.abyss.half === "first" ? "second" : "first";
      void mutate("abyss-half", () => mainDpsClient.selectAbyssHalf(next));
    }
  });

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => handleShortcut(event);
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  if (snapshot === null) {
    return (
      <div className="main-dps-root motion-route-loading grid place-items-center">
        <LoaderCircle
          className="size-8 animate-spin text-muted-foreground"
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
            message={t("Session reset. Use Undo within 5 seconds.")}
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
        <ConfirmationDialog
          kind={confirmation.kind}
          onCancel={() => setConfirmation(null)}
          onConfirm={confirmAction}
        />
      )}
      <MainUpdatePrompt />
      {!snapshot.onboarding.done && (
        <OnboardingOverlay
          onboarding={snapshot.onboarding}
          pending={pending !== null}
          onStep={(step) =>
            void mutate("onboarding-step", () =>
              mainDpsClient.setOnboardingStep(step),
            )
          }
          onFinish={(hudPreset) =>
            void mutate("onboarding-finish", () =>
              mainDpsClient.finishOnboarding(hudPreset),
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
                "bg-success",
              mainCaptureStatusTone(snapshot.capture.phase) === "transition" &&
                "bg-warning",
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

      <DataSourceControl disabled={pending !== null} />

      {snapshot.selectedRoundId === null &&
      (captureRunning ||
        snapshot.replayRunning ||
        snapshot.hasLiveSessionData) &&
      snapshot.dpsTime.warningMessageKey ? (
        <div
          className="border-b border-warning/25 bg-warning/10 px-3 py-1.5 text-xs text-warning"
          role="status"
        >
          {t(snapshot.dpsTime.warningMessageKey)}
        </div>
      ) : null}

      <main className="main-dps-content">
        {!abyssCollapsed && (
          <MainDpsToolbar
            snapshot={snapshot}
            pending={pending}
            mutate={mutate}
            effect={effect}
            startCapture={() => startCapture()}
            resetSession={() => resetSession()}
            onCollapse={() => setAbyssCollapsed(true)}
          />
        )}

        {mainDpsContentState(snapshot.replayRunning) === "replay-loading" ? (
          <ReplayImportLoading />
        ) : (
          <>
            <RoundSelector
              snapshot={snapshot}
              mutate={mutate}
              abyssCollapsed={abyssCollapsed}
              onExpand={() => setAbyssCollapsed(false)}
            />

            {snapshot.readout.abyss.detected && !abyssCollapsed && (
              <AbyssBar snapshot={snapshot} mutate={mutate} />
            )}

            {snapshot.display.metrics.length > 0 && (
              <Metrics snapshot={snapshot} />
            )}

            <MainDpsTeamSection
              snapshot={snapshot}
              hiddenCharacters={hiddenCharacters}
              onHideCharacter={(id) =>
                setHiddenCharacters((hidden) => new Set(hidden).add(id))
              }
              onShowAll={() => setHiddenCharacters(new Set())}
              pending={pending}
              apply={apply}
              effect={effect}
              startCapture={() => startCapture()}
              importReplay={importReplay}
            />
          </>
        )}
      </main>
    </div>
  );
}

function ConfirmationDialog({
  kind,
  onCancel,
  onConfirm,
}: {
  kind: MainDpsConfirmation["kind"];
  onCancel(): void;
  onConfirm(): void;
}) {
  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
    >
      <AlertDialogPortal>
        <AlertDialogBackdrop className="z-[70]" />
        <AlertDialogPopup className="top-1/2 left-1/2 z-[71] w-[calc(100vw-2rem)] max-w-md -translate-x-1/2 -translate-y-1/2 rounded-xl border bg-background p-5 shadow-2xl">
          <AlertDialogTitle className="text-sm">
            {t(
              kind === "start"
                ? "Confirm Start"
                : kind === "reset"
                  ? "Confirm Reset"
                  : "Confirm Import",
            )}
          </AlertDialogTitle>
          <AlertDialogDescription className="mt-2">
            {t(
              kind === "start"
                ? "Starting live capture clears the current stats and re-detects the game connection."
                : kind === "reset"
                  ? "This stops the current task and clears this session's stats, abyss state and detail caches."
                  : "Importing a replay stops the current task and clears existing stats.",
            )}
          </AlertDialogDescription>
          <div className="mt-5 flex justify-end gap-2">
            <AlertDialogClose render={<Button variant="outline" />}>
              {t("Cancel")}
            </AlertDialogClose>
            <Button onClick={onConfirm}>
              {t(
                kind === "start"
                  ? "Start"
                  : kind === "reset"
                    ? "Reset"
                    : "Import",
              )}
            </Button>
          </div>
        </AlertDialogPopup>
      </AlertDialogPortal>
    </AlertDialog>
  );
}

function RoundSelector({
  snapshot,
  mutate,
  abyssCollapsed,
  onExpand,
}: {
  snapshot: MainDpsSnapshot;
  mutate: MainDpsControls["mutate"];
  abyssCollapsed: boolean;
  onExpand(): void;
}) {
  const selectedIndex = snapshot.rounds.findIndex(
    (round) => round.id === snapshot.selectedRoundId,
  );
  return (
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
      <NativeSelect
        className="h-7 w-auto px-2"
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
      </NativeSelect>
      <Button
        size="icon-sm"
        variant="outline"
        aria-label={t("Next round")}
        disabled={
          selectedIndex < 0 || selectedIndex >= snapshot.rounds.length - 1
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
        <Button size="sm" variant="outline" onClick={onExpand}>
          {t("Expand")}
        </Button>
      )}
    </section>
  );
}

function AbyssBar({
  snapshot,
  mutate,
}: {
  snapshot: MainDpsSnapshot;
  mutate: MainDpsControls["mutate"];
}) {
  return (
    <section className="flex min-h-7 items-center gap-2">
      <strong className="whitespace-nowrap text-sm font-medium">
        {snapshot.readout.abyss.floor === null
          ? t("Abyss")
          : tf("Abyss Floor {}", [String(snapshot.readout.abyss.floor)])}
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
          <span className="text-sm font-medium text-success">
            {t("Challenge Cleared")}
          </span>
        </>
      )}
    </section>
  );
}

function Metrics({ snapshot }: { snapshot: MainDpsSnapshot }) {
  return (
    <section className="main-dps-metrics grid grid-cols-[repeat(auto-fit,minmax(7rem,1fr))] gap-1.5">
      {snapshot.display.metrics.includes("team_dps") && (
        <Metric
          value={snapshot.readout.summary.teamDps}
          format={formatMainMetric}
          label={t("Team DPS")}
        />
      )}
      {snapshot.display.metrics.includes("total_damage") && (
        <Metric
          value={snapshot.readout.summary.totalDamage}
          format={formatMainMetric}
          label={t("Total Damage")}
        />
      )}
      {snapshot.display.metrics.includes("total_damage_taken") && (
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
        <LoaderCircle
          className="size-8 animate-spin text-muted-foreground"
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
