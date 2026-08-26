import type { KeyboardEvent } from "react";

import type { HotkeyBinding } from "@/lib/tauri/settings-contract";

export function bindingFromKeyboardEvent(
  event: KeyboardEvent<HTMLButtonElement>,
): HotkeyBinding | null {
  if (event.repeat || event.metaKey) return null;
  const key = hotkeyKeyFromKeyboardEvent(event);
  if (key === null || (event.altKey && key === "F4")) return null;
  return {
    ctrl: event.ctrlKey,
    alt: event.altKey,
    shift: event.shiftKey,
    key,
  };
}

function hotkeyKeyFromKeyboardEvent(
  event: KeyboardEvent<HTMLButtonElement>,
): HotkeyBinding["key"] | null {
  if (/^Key[A-Z]$/.test(event.code)) {
    return event.code.slice(3) as HotkeyBinding["key"];
  }
  if (/^Digit[0-9]$/.test(event.code)) {
    return event.code.slice(5) as HotkeyBinding["key"];
  }
  if (/^F(?:[1-9]|1[0-2])$/.test(event.code)) {
    return event.code as HotkeyBinding["key"];
  }
  const namedKeys: Partial<Record<string, HotkeyBinding["key"]>> = {
    Home: "Home",
    End: "End",
    Insert: "Insert",
    Delete: "Delete",
    PageUp: "PageUp",
    PageDown: "PageDown",
    ArrowUp: "ArrowUp",
    ArrowDown: "ArrowDown",
    ArrowLeft: "ArrowLeft",
    ArrowRight: "ArrowRight",
    Space: "Space",
  };
  return namedKeys[event.code] ?? null;
}
