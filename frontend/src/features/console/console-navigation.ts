export const CONSOLE_PAGE_IDS = [
  "settings",
  "history",
  "timeline",
  "skills",
  "empty-curtain",
  "mod-studio",
  "character-data",
  "encrypted-ini",
  "packets",
  "resources",
  "diagnostics",
] as const;

export type ConsolePageId = (typeof CONSOLE_PAGE_IDS)[number];

export const DEFAULT_CONSOLE_PAGE: ConsolePageId = "settings";
export const CONSOLE_SIDEBAR_STORAGE_KEY = "nte.console.sidebar-collapsed.v1";

interface ConsolePreferenceStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export type ConsoleShortcut = "command-palette" | "previous-page" | "next-page";

interface ConsoleShortcutInput {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  repeat: boolean;
  editable: boolean;
}

export function isConsolePageId(value: string): value is ConsolePageId {
  return CONSOLE_PAGE_IDS.includes(value as ConsolePageId);
}

export function consolePageActivityMode(
  activePage: ConsolePageId,
  page: ConsolePageId,
): "visible" | "hidden" {
  return activePage === page ? "visible" : "hidden";
}

export function adjacentConsolePage(
  activePage: ConsolePageId,
  offset: -1 | 1,
): ConsolePageId {
  const index = CONSOLE_PAGE_IDS.indexOf(activePage);
  return CONSOLE_PAGE_IDS[
    (index + offset + CONSOLE_PAGE_IDS.length) % CONSOLE_PAGE_IDS.length
  ];
}

export function resolveConsoleShortcut({
  key,
  ctrlKey,
  altKey,
  shiftKey,
  repeat,
  editable,
}: ConsoleShortcutInput): ConsoleShortcut | null {
  if (repeat || altKey || shiftKey || !ctrlKey) return null;
  if (key.toLocaleLowerCase() === "k") return "command-palette";
  if (editable) return null;
  if (key === "PageUp") return "previous-page";
  if (key === "PageDown") return "next-page";
  return null;
}

export function isEditableKeyboardTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement
  );
}

export function readConsoleSidebarCollapsed(
  storage: ConsolePreferenceStorage | null = browserStorage(),
): boolean {
  if (storage === null) return false;
  try {
    return storage.getItem(CONSOLE_SIDEBAR_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

export function writeConsoleSidebarCollapsed(
  collapsed: boolean,
  storage: ConsolePreferenceStorage | null = browserStorage(),
): void {
  if (storage === null) return;
  try {
    storage.setItem(CONSOLE_SIDEBAR_STORAGE_KEY, String(collapsed));
  } catch {
    // A blocked WebView storage area only disables this display preference.
  }
}

function browserStorage(): ConsolePreferenceStorage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}
