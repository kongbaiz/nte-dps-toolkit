import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBackdrop,
  DialogPopup,
  DialogPortal,
  DialogTitle,
} from "@/components/ui/dialog";
import { t, tf } from "@/lib/i18n";
import type { HudPreset } from "@/lib/tauri/generated/HudPreset";
import type { MainDpsSnapshot } from "@/lib/tauri/generated/MainDpsSnapshot";
import { cn } from "@/lib/utils";

export function OnboardingOverlay({
  onboarding,
  pending,
  onStep,
  onFinish,
}: {
  onboarding: MainDpsSnapshot["onboarding"];
  pending: boolean;
  onStep(step: number): void;
  onFinish(hudPreset: HudPreset): void;
}) {
  const [hudPreset, setHudPreset] = useState<HudPreset>("standard");
  const finish = () => onFinish(hudPreset);
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
                  <p className="text-warning">
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
                      onClick={() => setHudPreset(preset)}
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
            <Button variant="ghost" disabled={pending} onClick={finish}>
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
                onClick={step === 3 ? finish : () => onStep(step + 1)}
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
