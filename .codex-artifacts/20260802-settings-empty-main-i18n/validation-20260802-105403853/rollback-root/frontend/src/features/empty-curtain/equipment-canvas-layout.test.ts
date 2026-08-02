import { describe, expect, it } from "vitest";

import {
  equipmentCanvasBackingStore,
  equipmentCanvasCell,
  equipmentCanvasHeaderMetrics,
  equipmentCanvasIndexAt,
  equipmentCanvasIndexAtContent,
  equipmentCanvasItemTop,
  equipmentCanvasLayout,
  equipmentCanvasRenderWindow,
  equipmentCanvasRenderWindowNeedsRefresh,
  equipmentCanvasVisibleStatCounts,
  equipmentStatLockColumnX,
  equipmentStatLockPresentation,
  visibleEquipmentCanvasCells,
} from "./equipment-canvas-layout";

describe("equipment canvas layout", () => {
  it("keeps hundreds of items in geometry instead of DOM-sized output", () => {
    const layout = equipmentCanvasLayout(1200, 331);
    expect(layout.columns).toBe(4);
    expect(layout.rows).toBe(83);
    expect(visibleEquipmentCanvasCells(layout, 331, 0, 700)).toHaveLength(12);
    expect(
      visibleEquipmentCanvasCells(layout, 331, layout.totalHeight - 700, 700)
        .length,
    ).toBeLessThanOrEqual(12);
  });

  it("maps pointer coordinates through the virtual scroll offset", () => {
    const layout = equipmentCanvasLayout(900, 20);
    const thirdRowTop = equipmentCanvasItemTop(layout, layout.columns * 2);
    expect(
      equipmentCanvasIndexAt(layout, 20, layout.padding + 1, 1, thirdRowTop),
    ).toBe(layout.columns * 2);
    expect(
      equipmentCanvasIndexAt(
        layout,
        20,
        layout.padding + layout.cardWidth + 1,
        1,
        0,
      ),
    ).toBeNull();
  });

  it("maps a hover overlay to stable content coordinates", () => {
    const layout = equipmentCanvasLayout(1200, 20);
    expect(equipmentCanvasCell(layout, layout.columns + 1)).toEqual({
      index: layout.columns + 1,
      x: layout.padding + layout.cardWidth + layout.gap,
      y: layout.padding + layout.cardHeight + layout.gap,
      width: layout.cardWidth,
      height: layout.cardHeight,
    });
  });

  it("maps pointer content coordinates without viewport position caching", () => {
    const layout = equipmentCanvasLayout(1200, 20);
    const index = layout.columns * 2 + 2;
    const cell = equipmentCanvasCell(layout, index);
    expect(
      equipmentCanvasIndexAtContent(
        layout,
        20,
        cell.x + cell.width / 2,
        cell.y + cell.height / 2,
      ),
    ).toBe(index);
  });

  it("shows unlock state only while a secondary stat is locked", () => {
    expect(equipmentStatLockPresentation(false, 15)).toEqual({
      locked: true,
      unlockLabel: "+15",
    });
    expect(equipmentStatLockPresentation(true, 15)).toEqual({
      locked: false,
      unlockLabel: null,
    });
  });

  it("keeps both module main stats and all four secondary stats visible", () => {
    expect(equipmentCanvasVisibleStatCounts(2, 4)).toEqual({
      main: 2,
      secondary: 4,
    });
    expect(equipmentCanvasVisibleStatCounts(4, 4)).toEqual({
      main: 4,
      secondary: 2,
    });
  });

  it("keeps every locked secondary stat icon in one fixed column", () => {
    const valueRightX = 300;
    const lockColumns = ["+5", "+10", "+15", "+20"].map(() =>
      equipmentStatLockColumnX(valueRightX),
    );
    expect(lockColumns).toEqual([252, 252, 252, 252]);
  });

  it("keeps an equipped character avatar inside the card header", () => {
    expect(equipmentCanvasHeaderMetrics(320, true)).toEqual({
      avatarX: 248,
      titleWidth: 166,
      levelWidth: 166,
    });
    expect(equipmentCanvasHeaderMetrics(320, false)).toEqual({
      avatarX: null,
      titleWidth: 202,
      levelWidth: 230,
    });
  });

  it("caps the sharp backing store at two device pixels per CSS pixel", () => {
    expect(equipmentCanvasBackingStore(900, 600, 1.5)).toEqual({
      width: 1350,
      height: 900,
    });
    expect(equipmentCanvasBackingStore(900, 600, 3)).toEqual({
      width: 1800,
      height: 1200,
    });
  });

  it("keeps scrolling inside a compositor-backed overscan window", () => {
    const layout = equipmentCanvasLayout(1200, 331);
    const renderWindow = equipmentCanvasRenderWindow(layout, 0, 700);
    expect(renderWindow).toEqual({ top: 0, height: 1860, overscan: 580 });
    expect(
      equipmentCanvasRenderWindowNeedsRefresh(
        renderWindow,
        layout.totalHeight,
        700,
        700,
      ),
    ).toBe(false);
    expect(
      equipmentCanvasRenderWindowNeedsRefresh(
        renderWindow,
        layout.totalHeight,
        1000,
        700,
      ),
    ).toBe(true);
  });
});
