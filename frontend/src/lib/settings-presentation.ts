import { create } from "zustand";

import { setFrontendLanguage } from "@/lib/i18n";
import { settingsPresentationEqual } from "@/lib/settings-presentation-model";
import type { InterfaceSettingsSnapshot } from "@/lib/tauri/generated/InterfaceSettingsSnapshot";
import { syncConsoleWindowBackground } from "@/lib/tauri/console-window-background";

const DEFAULT_PRESENTATION: InterfaceSettingsSnapshot = {
  language: "zh-CN",
  darkMode: false,
  themePreset: "zinc",
  accent: "zinc",
  density: "cozy",
  reduceMotion: false,
  islandNotifications: true,
  islandOffsetX: 0,
};
const STORAGE_KEY = "nte.settings.presentation.v1";

/** The interface settings this window renders with; Rust is the source of truth. */
export const useSettingsPresentation = create<InterfaceSettingsSnapshot>(
  readStoredPresentation,
);

useSettingsPresentation.subscribe((next) => {
  applyDocumentPresentation(next);
  setFrontendLanguage(next.language);
});

window.addEventListener("storage", (event) => {
  if (event.key !== STORAGE_KEY) return;
  replacePresentation(readStoredPresentation());
});

export function applySettingsPresentation(
  next: InterfaceSettingsSnapshot,
): void {
  if (replacePresentation(next)) {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  }
}

export function bootstrapSettingsPresentation(): void {
  const presentation = useSettingsPresentation.getState();
  applyDocumentPresentation(presentation);
  setFrontendLanguage(presentation.language);
}

function replacePresentation(next: InterfaceSettingsSnapshot): boolean {
  if (settingsPresentationEqual(useSettingsPresentation.getState(), next)) {
    return false;
  }
  useSettingsPresentation.setState(next, true);
  return true;
}

function applyDocumentPresentation(next: InterfaceSettingsSnapshot) {
  const root = document.documentElement;
  const isConsole = root.dataset.windowRoute === "console";
  const isOpaqueDesktop =
    isConsole ||
    root.dataset.windowRoute === "main-dps" ||
    root.dataset.windowRoute === "abyss-values" ||
    root.dataset.windowRoute === "combat-details";
  root.classList.toggle("dark", next.darkMode);
  root.classList.toggle("console-light", isOpaqueDesktop && !next.darkMode);
  root.dataset.themePreset = next.themePreset;
  root.dataset.accent = next.accent;
  root.dataset.density = next.density;
  root.classList.toggle("reduce-motion", next.reduceMotion);
  if (isOpaqueDesktop) {
    void syncConsoleWindowBackground(next.darkMode).catch((error: unknown) => {
      console.error("sync Console window background failed", error);
    });
  }
}

function readStoredPresentation(): InterfaceSettingsSnapshot {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "");
    if (
      typeof value === "object" &&
      value !== null &&
      "language" in value &&
      (value.language === "en" ||
        value.language === "ja" ||
        value.language === "zh-CN") &&
      "darkMode" in value &&
      typeof value.darkMode === "boolean"
    ) {
      return { ...DEFAULT_PRESENTATION, ...value } as InterfaceSettingsSnapshot;
    }
  } catch {
    // A malformed local projection is replaced by the Rust snapshot in Console.
  }
  return DEFAULT_PRESENTATION;
}
