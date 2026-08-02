import { describe, expect, it } from "vitest";

import type { HudModuleId } from "@/lib/tauri/technical-contract";

import {
  resolveHudModulePointerTarget,
  type HudModulePointerBounds,
} from "./use-hud-module-pointer-reorder";

const modules: HudModuleId[] = ["title", "status", "summary"];
const rows = new Map<HudModuleId, HudModulePointerBounds>([
  ["title", { left: 0, right: 200, top: 0, bottom: 40, height: 40 }],
  ["status", { left: 0, right: 200, top: 40, bottom: 80, height: 40 }],
  ["summary", { left: 0, right: 200, top: 80, bottom: 120, height: 40 }],
]);

describe("HUD pointer module reorder", () => {
  it("resolves vertical before and after targets while skipping the source row", () => {
    const boundsFor = (module: HudModuleId) => rows.get(module) ?? null;

    expect(
      resolveHudModulePointerTarget(modules, "title", 100, 45, boundsFor),
    ).toEqual({ module: "status", insertAfter: false });
    expect(
      resolveHudModulePointerTarget(modules, "title", 100, 75, boundsFor),
    ).toEqual({ module: "status", insertAfter: true });
    expect(
      resolveHudModulePointerTarget(modules, "status", 100, 60, boundsFor),
    ).toBeNull();
  });

  it("does not invent a target outside the one-column module list", () => {
    expect(
      resolveHudModulePointerTarget(
        modules,
        "title",
        220,
        60,
        (module) => rows.get(module) ?? null,
      ),
    ).toBeNull();
  });
});
