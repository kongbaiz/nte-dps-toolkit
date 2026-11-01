import type { EmptyCurtainItemSnapshot } from "@/lib/tauri/generated/EmptyCurtainItemSnapshot";

export type KnownEquipmentQuality = "blue" | "purple" | "orange";

export interface EquipmentFilters {
  filterIds: string[];
  qualities: KnownEquipmentQuality[];
  characterIds: number[];
  mainstats: string[];
  substats: string[];
}

export function emptyEquipmentFilters(): EquipmentFilters {
  return {
    filterIds: [],
    qualities: [],
    characterIds: [],
    mainstats: [],
    substats: [],
  };
}

export function activeEquipmentFilterCount(filters: EquipmentFilters): number {
  return (
    filters.filterIds.length +
    filters.qualities.length +
    filters.characterIds.length +
    filters.mainstats.length +
    filters.substats.length
  );
}

export function filterEquipmentItems(
  items: EmptyCurtainItemSnapshot[],
  filters: EquipmentFilters,
): EmptyCurtainItemSnapshot[] {
  const filterIds = new Set(filters.filterIds);
  const qualities = new Set<string>(filters.qualities);
  const characterIds = new Set(filters.characterIds);
  return items.filter(
    (item) =>
      (filterIds.size === 0 || filterIds.has(item.filterId)) &&
      (qualities.size === 0 ||
        (item.quality !== null && qualities.has(item.quality))) &&
      (characterIds.size === 0 ||
        (item.equippedCharacterId !== null &&
          characterIds.has(item.equippedCharacterId))) &&
      (filters.mainstats.length === 0 ||
        filters.mainstats.some((property) =>
          item.stats.some((stat) => stat.main && stat.property === property),
        )) &&
      filters.substats.every((property) =>
        item.stats.some((stat) => !stat.main && stat.property === property),
      ),
  );
}

export function matchesEquipmentSearch(value: string, query: string): boolean {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  return (
    normalizedQuery.length === 0 ||
    value.toLocaleLowerCase().includes(normalizedQuery)
  );
}
