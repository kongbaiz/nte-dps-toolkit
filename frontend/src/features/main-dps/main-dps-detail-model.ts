import type {
  MainDpsDetailColumns,
  MainDpsDetailSnapshot,
} from "@/lib/tauri/main-dps-detail-contract";

export type DetailColumnKey =
  "time" | "character" | "type" | "damage" | "target";

export const DEFAULT_DETAIL_COLUMNS: MainDpsDetailColumns = {
  showTime: true,
  showCharacter: true,
  showType: true,
  showDamage: true,
  showTarget: true,
  timeWidth: 92,
  characterWidth: 132,
  typeWidth: 250,
  damageWidth: 130,
  targetWidth: 180,
};

export const DETAIL_ROW_HEIGHT = 64;

export function detailVisibleRowRange(
  rowCount: number,
  scrollTop: number,
  viewportHeight: number,
  overscan = 8,
): { start: number; end: number } {
  if (rowCount <= 0) return { start: 0, end: 0 };
  const first = Math.min(
    rowCount - 1,
    Math.floor(Math.max(0, scrollTop) / DETAIL_ROW_HEIGHT),
  );
  const visible = Math.ceil(Math.max(0, viewportHeight) / DETAIL_ROW_HEIGHT);
  return {
    start: Math.max(0, first - overscan),
    end: Math.min(rowCount, first + visible + overscan),
  };
}

export function setDetailColumnVisible(
  columns: MainDpsDetailColumns,
  column: DetailColumnKey,
  visible: boolean,
): MainDpsDetailColumns {
  return { ...columns, [visibilityField(column)]: visible };
}

export function setDetailColumnWidth(
  columns: MainDpsDetailColumns,
  column: DetailColumnKey,
  width: number,
): MainDpsDetailColumns {
  return {
    ...columns,
    [widthField(column)]: Math.round(Math.max(64, Math.min(600, width))),
  };
}

export function resetDetailColumnWidths(
  columns: MainDpsDetailColumns,
): MainDpsDetailColumns {
  return {
    ...columns,
    timeWidth: DEFAULT_DETAIL_COLUMNS.timeWidth,
    characterWidth: DEFAULT_DETAIL_COLUMNS.characterWidth,
    typeWidth: DEFAULT_DETAIL_COLUMNS.typeWidth,
    damageWidth: DEFAULT_DETAIL_COLUMNS.damageWidth,
    targetWidth: DEFAULT_DETAIL_COLUMNS.targetWidth,
  };
}

export function detailColumnVisible(
  columns: MainDpsDetailColumns,
  column: DetailColumnKey,
): boolean {
  return columns[visibilityField(column)];
}

export function detailColumnWidth(
  columns: MainDpsDetailColumns,
  column: DetailColumnKey,
): number {
  return columns[widthField(column)];
}

export function mergeLiveDetailSnapshot(
  current: MainDpsDetailSnapshot | null,
  next: MainDpsDetailSnapshot,
): MainDpsDetailSnapshot {
  if (current === null || !sameDetailView(current, next)) return next;
  if (current.rows.length <= next.rows.length) return next;
  if (
    next.rows.length === 0 ||
    current.rows[next.rows.length - 1]?.id !== next.rows.at(-1)?.id
  )
    return next;
  const retained = current.rows.slice(next.rows.length, next.totalHits);
  return { ...next, offset: 0, rows: [...next.rows, ...retained] };
}

export function mergePagedDetailSnapshot(
  current: MainDpsDetailSnapshot | null,
  next: MainDpsDetailSnapshot,
): MainDpsDetailSnapshot {
  if (current === null || !sameDetailView(current, next)) return next;
  const existingIds = new Set(current.rows.map((row) => row.id));
  const fresh = next.rows.filter((row) => !existingIds.has(row.id));
  return { ...next, offset: 0, rows: [...current.rows, ...fresh] };
}

function sameDetailView(
  left: MainDpsDetailSnapshot,
  right: MainDpsDetailSnapshot,
): boolean {
  return (
    left.kind === right.kind &&
    left.characterId === right.characterId &&
    left.filter === right.filter &&
    left.qteType === right.qteType &&
    left.skillFilter === right.skillFilter
  );
}

function visibilityField(
  column: DetailColumnKey,
): keyof Pick<
  MainDpsDetailColumns,
  "showTime" | "showCharacter" | "showType" | "showDamage" | "showTarget"
> {
  switch (column) {
    case "time":
      return "showTime";
    case "character":
      return "showCharacter";
    case "type":
      return "showType";
    case "damage":
      return "showDamage";
    case "target":
      return "showTarget";
  }
}

function widthField(
  column: DetailColumnKey,
): keyof Pick<
  MainDpsDetailColumns,
  "timeWidth" | "characterWidth" | "typeWidth" | "damageWidth" | "targetWidth"
> {
  switch (column) {
    case "time":
      return "timeWidth";
    case "character":
      return "characterWidth";
    case "type":
      return "typeWidth";
    case "damage":
      return "damageWidth";
    case "target":
      return "targetWidth";
  }
}
