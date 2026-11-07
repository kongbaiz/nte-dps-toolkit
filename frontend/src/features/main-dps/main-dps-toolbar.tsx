import { useState } from "react";

import { useWindowMotion } from "@/components/nte/window-motion-context";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverClose,
  PopoverPopup,
  PopoverPortal,
  PopoverPositioner,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useCloseOnWindowBlur } from "@/hooks/use-close-on-window-blur";
import { t, tf } from "@/lib/i18n";
import {
  applySettingsPresentation,
  useSettingsPresentation,
} from "@/lib/settings-presentation";
import { mainDpsClient } from "@/lib/tauri/main-dps-client";
import type { MainDpsSnapshot } from "@/lib/tauri/generated/MainDpsSnapshot";

import type { useMainDps } from "./use-main-dps";

type MainDpsControls = ReturnType<typeof useMainDps>;

export function MainDpsToolbar({
  snapshot,
  pending,
  mutate,
  effect,
  startCapture,
  resetSession,
  onCollapse,
}: {
  snapshot: MainDpsSnapshot;
  pending: string | null;
  mutate: MainDpsControls["mutate"];
  effect: MainDpsControls["effect"];
  startCapture(): void;
  resetSession(): Promise<void>;
  onCollapse(): void;
}) {
  const { transitionToWindow } = useWindowMotion();
  const captureRunning = snapshot.actions.canStopCapture;
  return (
    <section className="main-dps-toolbar flex flex-wrap items-center justify-between gap-1.5">
      <div className="flex flex-wrap gap-1.5 [@media(max-height:560px)]:flex-none">
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
          disabled={pending !== null || !snapshot.actions.canStartNewRound}
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
            {tf("Paused / {} pending events / {} debug packets", [
              snapshot.pausedPendingEvents,
              snapshot.pausedDebugPackets,
            ])}
          </span>
        )}
        {snapshot.readout.abyss.detected && (
          <Button size="sm" variant="outline" onClick={onCollapse}>
            {t("Collapse")}
          </Button>
        )}
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            void effect("hud", () => transitionToWindow(mainDpsClient.openHud))
          }
        >
          {t("HUD")}
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() => void effect("console", mainDpsClient.openConsole)}
        >
          {t("Console")}
        </Button>
      </div>
      <div className="relative flex gap-1.5 [@media(max-height:560px)]:flex-none">
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
        <AppearancePopover snapshot={snapshot} mutate={mutate} />
      </div>
    </section>
  );
}

function AppearancePopover({
  snapshot,
  mutate,
}: {
  snapshot: MainDpsSnapshot;
  mutate: MainDpsControls["mutate"];
}) {
  const presentation = useSettingsPresentation();
  const [appearanceOpen, setAppearanceOpen] = useState(false);
  useCloseOnWindowBlur(appearanceOpen, () => setAppearanceOpen(false));
  return (
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
                <span>{Math.round(snapshot.appearance.opacity * 100)}%</span>
              </span>
              <input
                type="range"
                min="35"
                max="100"
                defaultValue={Math.round(snapshot.appearance.opacity * 100)}
                onPointerUp={(event) =>
                  void mutate("appearance", () =>
                    mainDpsClient.setAppearance(
                      snapshot.appearance.darkMode,
                      Number(event.currentTarget.value) / 100,
                    ),
                  )
                }
                onKeyUp={(event) => {
                  if (event.key === "ArrowLeft" || event.key === "ArrowRight")
                    void mutate("appearance", () =>
                      mainDpsClient.setAppearance(
                        snapshot.appearance.darkMode,
                        Number(event.currentTarget.value) / 100,
                      ),
                    );
                }}
              />
            </label>
            <PopoverClose className="sr-only">{t("Close")}</PopoverClose>
          </PopoverPopup>
        </PopoverPositioner>
      </PopoverPortal>
    </Popover>
  );
}
