import { useSyncExternalStore } from "react";

import { setFrontendLanguage } from "@/lib/i18n";
import { settingsPresentationEqual } from "@/lib/settings-presentation-model";
import type { InterfaceSettings } from "@/lib/tauri/settings-contract";

const DEFAULT_PRESENTATION: InterfaceSettings = {
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
let presentation = readStoredPresentation();
let revision = 0;
const listeners = new Set<() => void>();

window.addEventListener("storage", (event) => {
  if (event.key !== STORAGE_KEY) return;
  const next = readStoredPresentation();
  if (settingsPresentationEqual(presentation, next)) return;
  presentation = next;
  applyDocumentPresentation(next);
  setFrontendLanguage(next.language);
  revision += 1;
  listeners.forEach((listener) => listener());
});

export function applySettingsPresentation(next: InterfaceSettings): void {
  if (settingsPresentationEqual(presentation, next)) return;
  presentation = next;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  applyDocumentPresentation(next);
  setFrontendLanguage(next.language);
  revision += 1;
  listeners.forEach((listener) => listener());
}

export function bootstrapSettingsPresentation(): void {
  applyDocumentPresentation(presentation);
  setFrontendLanguage(presentation.language);
}

function applyDocumentPresentation(next: InterfaceSettings) {
  const root = document.documentElement;
  root.classList.toggle("dark", next.darkMode);
  root.dataset.themePreset = next.themePreset;
  root.dataset.accent = next.accent;
  root.dataset.density = next.density;
  root.classList.toggle("reduce-motion", next.reduceMotion);
}

export function useSettingsPresentation(): InterfaceSettings {
  useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => revision,
    () => revision,
  );
  return presentation;
}

function readStoredPresentation(): InterfaceSettings {
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
      return { ...DEFAULT_PRESENTATION, ...value } as InterfaceSettings;
    }
  } catch {
    // A malformed local projection is replaced by the Rust snapshot in Console.
  }
  return DEFAULT_PRESENTATION;
}
