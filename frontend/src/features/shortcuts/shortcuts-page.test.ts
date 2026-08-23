import { describe, expect, it } from "vitest";
import type { KeyboardEvent } from "react";

import { bindingFromKeyboardEvent } from "./shortcut-binding";

function keyEvent(
  code: string,
  overrides: Partial<KeyboardEvent<HTMLButtonElement>> = {},
): KeyboardEvent<HTMLButtonElement> {
  return {
    code,
    key: code,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    repeat: false,
    ...overrides,
  } as KeyboardEvent<HTMLButtonElement>;
}

describe("shortcut recording", () => {
  it("records plain, modifier and navigation bindings", () => {
    expect(bindingFromKeyboardEvent(keyEvent("KeyK"))).toEqual({
      ctrl: false,
      alt: false,
      shift: false,
      key: "K",
    });
    expect(
      bindingFromKeyboardEvent(
        keyEvent("Home", { ctrlKey: true, shiftKey: true }),
      ),
    ).toEqual({
      ctrl: true,
      alt: false,
      shift: true,
      key: "Home",
    });
  });

  it("rejects unsupported, repeated and Windows-reserved bindings", () => {
    expect(bindingFromKeyboardEvent(keyEvent("Escape"))).toBeNull();
    expect(
      bindingFromKeyboardEvent(keyEvent("KeyA", { repeat: true })),
    ).toBeNull();
    expect(
      bindingFromKeyboardEvent(keyEvent("F4", { altKey: true })),
    ).toBeNull();
  });
});
