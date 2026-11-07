import type { KeyboardEvent } from "react";

import type { HotkeyBindingSnapshot } from "@/lib/tauri/generated/HotkeyBindingSnapshot";

export function bindingFromKeyboardEvent(
  event: KeyboardEvent<HTMLButtonElement>,
): HotkeyBindingSnapshot | null {
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

export function formatHotkeyBinding(
  binding: HotkeyBindingSnapshot | null,
): string | null {
  if (binding === null) return null;
  return [
    binding.ctrl ? "Ctrl" : null,
    binding.alt ? "Alt" : null,
    binding.shift ? "Shift" : null,
    binding.key,
  ]
    .filter((part): part is string => part !== null)
    .join("+");
}

function hotkeyKeyFromKeyboardEvent(
  event: KeyboardEvent<HTMLButtonElement>,
): HotkeyBindingSnapshot["key"] | null {
  if (/^Key[A-Z]$/.test(event.code)) {
    return event.code.slice(3) as HotkeyBindingSnapshot["key"];
  }
  if (/^Digit[0-9]$/.test(event.code)) {
    return event.code.slice(5) as HotkeyBindingSnapshot["key"];
  }
  if (/^F(?:[1-9]|1[0-2])$/.test(event.code)) {
    return event.code as HotkeyBindingSnapshot["key"];
  }
  const namedKeys: Partial<Record<string, HotkeyBindingSnapshot["key"]>> = {
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
