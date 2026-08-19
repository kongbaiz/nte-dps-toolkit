import { useSyncExternalStore } from "react";

import japanese from "@res/languages/ja.json";
import simplifiedChinese from "@res/languages/zh-CN.json";

import type { SettingsLanguage } from "@/lib/tauri/settings-contract";

const dictionaries: Record<
  Exclude<SettingsLanguage, "en">,
  Record<string, string>
> = {
  ja: {
    ...japanese,
    "Follow-up": "追撃",
  },
  "zh-CN": {
    ...simplifiedChinese,
    "Follow-up": "追击",
  },
};
let language: SettingsLanguage = "zh-CN";
let revision = 0;
const listeners = new Set<() => void>();

export function t(key: string): string {
  return language === "en" ? key : (dictionaries[language][key] ?? key);
}

export function tf(key: string, arguments_: readonly string[]): string {
  return formatTemplate(t(key), arguments_);
}

/**
 * Formats the shared Rust/TypeScript placeholder grammar. `{n}` reuses argument
 * n anywhere in the template; when the current argument has no indexed token,
 * it consumes the next `{}` token. Unknown or unfilled tokens stay literal.
 */
export function formatTemplate(
  template: string,
  arguments_: readonly string[],
): string {
  return arguments_.reduce((message, argument, index) => {
    const indexed = `{${index}}`;
    if (message.includes(indexed)) {
      return message.replaceAll(indexed, argument);
    }
    return message.replace("{}", argument);
  }, template);
}

export function setFrontendLanguage(next: SettingsLanguage): void {
  if (language === next) {
    return;
  }
  language = next;
  revision += 1;
  listeners.forEach((listener) => listener());
}

export function currentFrontendLanguage(): SettingsLanguage {
  return language;
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
