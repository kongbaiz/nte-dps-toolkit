import simplifiedChinese from "@res/languages/zh-CN.json";

const translations: Record<string, string> = simplifiedChinese;

export function t(key: string): string {
  return translations[key] ?? key;
}

export function tf(key: string, arguments_: readonly string[]): string {
  return arguments_.reduce(
    (message, argument, index) => message.replaceAll(`{${index}}`, argument),
    t(key),
  );
}
