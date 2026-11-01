import { create } from "zustand";

import japanese from "@res/languages/ja.json";
import simplifiedChinese from "@res/languages/zh-CN.json";

import type { Language } from "@/lib/tauri/generated/Language";

const dictionaries: Record<Exclude<Language, "en">, Record<string, string>> = {
  ja: {
    ...japanese,
    "Follow-up": "追撃",
  },
  "zh-CN": {
    ...simplifiedChinese,
    "Follow-up": "追击",
  },
};
/** The frontend language; components that render `t` output subscribe to it. */
export const useFrontendLanguage = create<Language>(() => "zh-CN");

export function t(key: string): string {
  const language = useFrontendLanguage.getState();
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

export function setFrontendLanguage(next: Language): void {
  useFrontendLanguage.setState(next, true);
}
