export const CONSOLE_PAGE_IDS = ["settings", "mod-studio"] as const;

export type ConsolePageId = (typeof CONSOLE_PAGE_IDS)[number];

export function isConsolePageId(value: string): value is ConsolePageId {
  return CONSOLE_PAGE_IDS.includes(value as ConsolePageId);
}
