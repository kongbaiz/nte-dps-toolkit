import type {
  ResourceCategory,
  ResourceItemSnapshot,
  ResourceSeverity,
} from "@/lib/tauri/resources-contract";

export type ResourceSeverityFilter = "all" | ResourceSeverity;
export type ResourceCategoryFilter = "all" | ResourceCategory;
export type ResourcesContentKind =
  "loading" | "error" | "clear" | "filtered-empty" | "list";

export function filterResourceItems(
  items: readonly ResourceItemSnapshot[],
  severity: ResourceSeverityFilter,
  category: ResourceCategoryFilter,
): ResourceItemSnapshot[] {
  return items.filter(
    (item) =>
      (severity === "all" || item.severity === severity) &&
      (category === "all" || item.category === category),
  );
}

export function resourcesContentKind(
  status: "loading" | "error" | "ready",
  totalItems: number,
  filteredItems: number,
): ResourcesContentKind {
  if (status === "loading") return "loading";
  if (status === "error") return "error";
  if (totalItems === 0) return "clear";
  return filteredItems === 0 ? "filtered-empty" : "list";
}
