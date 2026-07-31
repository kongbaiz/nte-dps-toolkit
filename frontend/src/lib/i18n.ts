import { useSyncExternalStore } from "react";

import japanese from "@res/languages/ja.json";
import simplifiedChinese from "@res/languages/zh-CN.json";

import type { SettingsLanguage } from "@/lib/tauri/settings-contract";

const dictionaries: Record<
  Exclude<SettingsLanguage, "en">,
  Record<string, string>
> = {
  ja: japanese,
  "zh-CN": simplifiedChinese,
};
let language: SettingsLanguage = "zh-CN";
let revision = 0;
const listeners = new Set<() => void>();

export function t(key: string): string {
  return language === "en" ? key : (dictionaries[language][key] ?? key);
}

export function tf(key: string, arguments_: readonly string[]): string {
  return arguments_.reduce((message, argument, index) => {
    const indexed = `{${index}}`;
    if (message.includes(indexed)) {
      return message.replaceAll(indexed, argument);
    }
    return message.replace("{}", argument);
  }, t(key));
}

export function setFrontendLanguage(next: SettingsLanguage): void {
  if (language === next) {
    return;
  }
  language = next;
  revision += 1;
  listeners.forEach((listener) => listener());
}

export function useTranslationRevision(): number {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => revision,
    () => revision,
  );
}
