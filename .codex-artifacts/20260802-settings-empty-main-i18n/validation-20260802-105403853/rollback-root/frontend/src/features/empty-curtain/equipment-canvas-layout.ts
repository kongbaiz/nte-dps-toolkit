export const EQUIPMENT_CANVAS_GAP = 10;
export const EQUIPMENT_CANVAS_PADDING = 10;
export const EQUIPMENT_CARD_HEIGHT = 280;
export const EQUIPMENT_CARD_MIN_WIDTH = 272;
export const EQUIPMENT_CARD_MAX_STAT_ROWS = 6;
export const EQUIPMENT_STAT_LOCK_RIGHT_OFFSET = 48;

export interface EquipmentCanvasLayout {
  columns: number;
  rows: number;
  cardWidth: number;
  cardHeight: number;
  gap: number;
  padding: number;
  totalHeight: number;
}

export interface EquipmentCanvasCell {
  index: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface EquipmentStatLockPresentation {
  locked: boolean;
  unlockLabel: string | null;
}

export interface EquipmentCanvasVisibleStatCounts {
  main: number;
  secondary: number;
}

export interface EquipmentCanvasBackingStore {
  width: number;
  height: number;
}

export interface EquipmentCanvasHeaderMetrics {
  avatarX: number | null;
  titleWidth: number;
  levelWidth: number;
}

export interface EquipmentCanvasRenderWindow {
  top: number;
  height: number;
  overscan: number;
}

export function equipmentCanvasBackingStore(
  width: number,
  height: number,
  devicePixelRatio: number,
): EquipmentCanvasBackingStore {
  const ratio = Math.min(2, Math.max(1, devicePixelRatio || 1));
  return {
    width: Math.ceil(Math.max(1, width) * ratio),
    height: Math.ceil(Math.max(1, height) * ratio),
  };
}

export function equipmentCanvasHeaderMetrics(
  width: number,
  hasEquippedAvatar: boolean,
): EquipmentCanvasHeaderMetrics {
  return {
    avatarX: hasEquippedAvatar ? width - 72 : null,
    titleWidth: Math.max(36, width - 74 - (hasEquippedAvatar ? 80 : 44)),
    levelWidth: Math.max(36, width - 74 - (hasEquippedAvatar ? 80 : 16)),
  };
}

export function equipmentCanvasRenderWindow(
  layout: EquipmentCanvasLayout,
  scrollTop: number,
  viewportHeight: number,
): EquipmentCanvasRenderWindow {
  const safeViewportHeight = Math.max(1, viewportHeight);
  const stride = layout.cardHeight + layout.gap;
  const overscan = stride * 2;
  const height = Math.min(
    layout.totalHeight,
    safeViewportHeight + overscan * 2,
  );
  return {
    top: Math.max(
      0,
      Math.min(
        Math.max(0, scrollTop - overscan),
        Math.max(0, layout.totalHeight - height),
      ),
    ),
    height,
    overscan,
  };
}

export function equipmentCanvasRenderWindowNeedsRefresh(
  renderWindow: EquipmentCanvasRenderWindow,
  totalHeight: number,
  scrollTop: number,
  viewportHeight: number,
): boolean {
  const threshold = renderWindow.overscan * 0.35;
  const distanceFromTop = scrollTop - renderWindow.top;
  const distanceFromBottom =
    renderWindow.top + renderWindow.height - (scrollTop + viewportHeight);
  return (
    (renderWindow.top > 0 && distanceFromTop < threshold) ||
    (renderWindow.top + renderWindow.height < totalHeight &&
      distanceFromBottom < threshold)
  );
}

export function equipmentStatLockPresentation(
  unlocked: boolean,
  unlockLevel: number | null,
): EquipmentStatLockPresentation {
  if (unlocked) return { locked: false, unlockLabel: null };
  return {
    locked: true,
    unlockLabel: unlockLevel === null ? null : `+${unlockLevel}`,
  };
}

export function equipmentStatLockColumnX(valueRightX: number): number {
  return valueRightX - EQUIPMENT_STAT_LOCK_RIGHT_OFFSET;
}

export function equipmentCanvasVisibleStatCounts(
  mainCount: number,
  secondaryCount: number,
): EquipmentCanvasVisibleStatCounts {
  const main = Math.min(Math.max(0, mainCount), EQUIPMENT_CARD_MAX_STAT_ROWS);
  return {
    main,
    secondary: Math.min(
      Math.max(0, secondaryCount),
      EQUIPMENT_CARD_MAX_STAT_ROWS - main,
    ),
  };
}

export function equipmentCanvasLayout(
  width: number,
  itemCount: number,
): EquipmentCanvasLayout {
  const safeWidth = Math.max(1, width);
  const innerWidth = Math.max(1, safeWidth - EQUIPMENT_CANVAS_PADDING * 2);
  const columns = Math.max(
    1,
    Math.floor(
      (innerWidth + EQUIPMENT_CANVAS_GAP) /
        (EQUIPMENT_CARD_MIN_WIDTH + EQUIPMENT_CANVAS_GAP),
    ),
  );
  const rows = Math.ceil(Math.max(0, itemCount) / columns);
  const cardWidth =
    (innerWidth - EQUIPMENT_CANVAS_GAP * (columns - 1)) / columns;
  return {
    columns,
    rows,
    cardWidth,
    cardHeight: EQUIPMENT_CARD_HEIGHT,
    gap: EQUIPMENT_CANVAS_GAP,
    padding: EQUIPMENT_CANVAS_PADDING,
    totalHeight:
      EQUIPMENT_CANVAS_PADDING * 2 +
      Math.max(
        0,
        rows * EQUIPMENT_CARD_HEIGHT + (rows - 1) * EQUIPMENT_CANVAS_GAP,
      ),
  };
}

export function visibleEquipmentCanvasCells(
  layout: EquipmentCanvasLayout,
  itemCount: number,
  scrollTop: number,
  viewportHeight: number,
): EquipmentCanvasCell[] {
  if (itemCount <= 0 || viewportHeight <= 0) return [];
  const stride = layout.cardHeight + layout.gap;
  const firstRow = Math.max(
    0,
    Math.floor((scrollTop - layout.padding) / stride),
  );
  const lastRow = Math.min(
    layout.rows - 1,
    Math.floor((scrollTop + viewportHeight - layout.padding) / stride),
  );
  const cells: EquipmentCanvasCell[] = [];
  for (let row = firstRow; row <= lastRow; row += 1) {
    for (let column = 0; column < layout.columns; column += 1) {
      const index = row * layout.columns + column;
      if (index >= itemCount) break;
      cells.push(equipmentCanvasCell(layout, index));
    }
  }
  return cells;
}

export function equipmentCanvasCell(
  layout: EquipmentCanvasLayout,
  index: number,
): EquipmentCanvasCell {
  const row = Math.floor(index / layout.columns);
  const column = index % layout.columns;
  return {
    index,
    x: layout.padding + column * (layout.cardWidth + layout.gap),
    y: layout.padding + row * (layout.cardHeight + layout.gap),
    width: layout.cardWidth,
    height: layout.cardHeight,
  };
}

export function equipmentCanvasIndexAt(
  layout: EquipmentCanvasLayout,
  itemCount: number,
  x: number,
  viewportY: number,
  scrollTop: number,
): number | null {
  return equipmentCanvasIndexAtContent(
    layout,
    itemCount,
    x,
    viewportY + scrollTop,
  );
}

export function equipmentCanvasIndexAtContent(
  layout: EquipmentCanvasLayout,
  itemCount: number,
  x: number,
  y: number,
): number | null {
  const contentX = x - layout.padding;
  const contentY = y - layout.padding;
  if (contentX < 0 || contentY < 0) return null;
  const columnStride = layout.cardWidth + layout.gap;
  const rowStride = layout.cardHeight + layout.gap;
  const column = Math.floor(contentX / columnStride);
  const row = Math.floor(contentY / rowStride);
  if (column >= layout.columns) return null;
  if (contentX % columnStride > layout.cardWidth) return null;
  if (contentY % rowStride > layout.cardHeight) return null;
  const index = row * layout.columns + column;
  return index < itemCount ? index : null;
}

export function equipmentCanvasItemTop(
  layout: EquipmentCanvasLayout,
  index: number,
): number {
  return (
    layout.padding +
    Math.floor(index / layout.columns) * (layout.cardHeight + layout.gap)
  );
}
