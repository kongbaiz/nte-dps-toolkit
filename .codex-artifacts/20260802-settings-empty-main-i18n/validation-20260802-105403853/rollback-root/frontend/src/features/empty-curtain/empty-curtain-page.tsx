import {
  Backpack,
  Check,
  Download,
  Filter,
  LockKeyhole,
  PackageOpen,
  RotateCcw,
  Search,
  Trash2,
  Upload,
  UsersRound,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { characterAvatarUrl } from "@/lib/character-avatar";
import { t, tf } from "@/lib/i18n";
import type { ManageItemInput } from "@/lib/tauri/empty-curtain-client";
import {
  itemUidKey,
  type EmptyCurtainCharacter,
  type EmptyCurtainCommandError,
  type EmptyCurtainItem,
  type EmptyCurtainPlacement,
  type ItemUid,
} from "@/lib/tauri/empty-curtain-contract";
import { cn } from "@/lib/utils";

import { EquipmentCanvasGrid } from "./equipment-canvas-grid";
import {
  activeEquipmentFilterCount,
  emptyEquipmentFilters,
  filterEquipmentItems,
  matchesEquipmentSearch,
  type EquipmentFilters,
  type KnownEquipmentQuality,
} from "./equipment-filters";
import { useEmptyCurtain } from "./use-empty-curtain";

const equipmentImages = import.meta.glob<string>(
  ["@res/images/kongmu/256/*.png", "@res/images/fangkuai/*.png"],
  { eager: true, query: "?url", import: "default" },
);
const imageByFileName = new Map(
  Object.entries(equipmentImages).map(([path, url]) => [fileName(path), url]),
);
const EMPTY_ITEMS: EmptyCurtainItem[] = [];

const QUALITY_OPTIONS: Array<{
  id: KnownEquipmentQuality;
  label: string;
  dotClassName: string;
}> = [
  { id: "blue", label: "Blue", dotClassName: "bg-sky-500" },
  { id: "purple", label: "Purple", dotClassName: "bg-violet-500" },
  { id: "orange", label: "Orange", dotClassName: "bg-amber-500" },
];

export function EmptyCurtainPage() {
  const model = useEmptyCurtain();
  const [filters, setFilters] = useState<EquipmentFilters>(
    emptyEquipmentFilters,
  );
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [charactersOpen, setCharactersOpen] = useState(false);
  const [selectedItemKey, setSelectedItemKey] = useState<string | null>(null);
  const items =
    model.state.status === "ready" ? model.state.snapshot.items : EMPTY_ITEMS;
  const visibleItems = useMemo(
    () => filterEquipmentItems(items, filters),
    [filters, items],
  );

  if (model.state.status === "loading") return <LoadingPage />;
  if (model.state.status === "error") {
    return <LoadError error={model.state.error} onRetry={model.retry} />;
  }

  const snapshot = model.state.snapshot;
  const activeFilters = activeEquipmentFilterCount(filters);
  const selectedItem = selectedItemKey
    ? (snapshot.items.find(
        (item) => itemUidKey(item.uid) === selectedItemKey,
      ) ?? null)
    : null;

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b px-3 py-3 min-[640px]:px-5">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-base font-semibold">{t("Console Loadout")}</h1>
            <Badge variant="outline" className="gap-1.5">
              <span className="size-1.5 rounded-full bg-emerald-500" />
              {t("Live")}
            </Badge>
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t("Console inventory follows the current capture")}
          </p>
        </div>
      </header>

      {model.notice ? (
        <FloatingNotice error={model.notice} onClose={model.clearNotice} />
      ) : null}

      <div className="min-h-0 flex-1 overflow-hidden">
        <div className="mx-auto flex h-full min-h-0 w-full max-w-[1700px] flex-col px-3 py-3 min-[900px]:px-4">
          {!snapshot.hasData ? (
            <Empty className="min-h-[28rem] flex-1 border border-dashed">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <PackageOpen aria-hidden="true" />
                </EmptyMedia>
                <EmptyTitle>{t("No Console equipment data")}</EmptyTitle>
                <EmptyDescription>
                  {t(
                    "Start capture or import a replay to inspect Console equipment",
                  )}
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : (
            <>
              <div className="flex flex-wrap items-center gap-2 border-y border-border/70 py-2.5">
                <Button
                  variant={filtersOpen ? "secondary" : "outline"}
                  size="sm"
                  aria-expanded={filtersOpen}
                  onClick={() => {
                    setCharactersOpen(false);
                    setFiltersOpen((open) => !open);
                  }}
                >
                  <Filter aria-hidden="true" />
                  {activeFilters > 0
                    ? tf("Filter ({})", [String(activeFilters)])
                    : t("Filter")}
                </Button>
                {activeFilters > 0 ? (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setFilters(emptyEquipmentFilters())}
                  >
                    <RotateCcw aria-hidden="true" />
                    {t("Clear Filters")}
                  </Button>
                ) : null}
                <span className="text-xs text-muted-foreground">
                  {tf("{} of {} items", [
                    String(visibleItems.length),
                    String(snapshot.items.length),
                  ])}
                  {!snapshot.complete
                    ? ` · ${t("Inventory snapshot is still being assembled")}`
                    : ""}
                </span>
                <div className="ml-auto flex flex-wrap items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={model.actionPending}
                    onClick={model.exportInventory}
                  >
                    <Download aria-hidden="true" />
                    {t("Export for Drive Calculator")}
                  </Button>
                  <Button
                    variant={charactersOpen ? "secondary" : "outline"}
                    size="sm"
                    disabled={
                      model.actionPending || snapshot.characters.length === 0
                    }
                    onClick={() => {
                      setFiltersOpen(false);
                      setCharactersOpen((open) => !open);
                    }}
                  >
                    <UsersRound aria-hidden="true" />
                    {t("Character Equipment")}
                  </Button>
                </div>
              </div>

              {charactersOpen ? (
                <CharacterEquipmentDialog
                  characters={snapshot.characters}
                  items={snapshot.items}
                  pending={model.actionPending}
                  onImport={model.importLoadout}
                  onExport={model.exportLoadout}
                  onAction={model.characterAction}
                  onDone={() => setCharactersOpen(false)}
                />
              ) : null}
              {filtersOpen ? (
                <FilterDialog
                  items={snapshot.items}
                  characters={snapshot.characters}
                  filters={filters}
                  onChange={setFilters}
                  onDone={() => setFiltersOpen(false)}
                />
              ) : null}

              <p className="py-2.5 text-xs text-muted-foreground">
                {t("Manage equipment from each item card through the plugin")}
              </p>
              {visibleItems.length === 0 ? (
                <Empty className="min-h-[24rem] flex-1 border">
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <PackageOpen aria-hidden="true" />
                    </EmptyMedia>
                    <EmptyTitle>
                      {t("No equipment matches the current filters")}
                    </EmptyTitle>
                    <EmptyDescription>{t("Clear Filters")}</EmptyDescription>
                  </EmptyHeader>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setFilters(emptyEquipmentFilters())}
                  >
                    <RotateCcw aria-hidden="true" />
                    {t("Clear Filters")}
                  </Button>
                </Empty>
              ) : (
                <EquipmentCanvasGrid
                  items={visibleItems}
                  imageUrl={equipmentImageUrl}
                  characterAvatarUrl={characterAvatarUrl}
                  onOpenItem={(item) =>
                    setSelectedItemKey(itemUidKey(item.uid))
                  }
                />
              )}
              {selectedItem ? (
                <EquipmentDetailsDialog
                  item={selectedItem}
                  characters={snapshot.characters}
                  pending={model.actionPending}
                  loadPositions={model.positions}
                  onManage={model.manageItem}
                  onDone={() => setSelectedItemKey(null)}
                />
              ) : null}
            </>
          )}
        </div>
      </div>
    </section>
  );
}

function CharacterEquipmentDialog({
  characters,
  items,
  pending,
  onImport,
  onExport,
  onAction,
  onDone,
}: {
  characters: EmptyCurtainCharacter[];
  items: EmptyCurtainItem[];
  pending: boolean;
  onImport: () => void;
  onExport: (uid: ItemUid) => void;
  onAction: (uid: ItemUid, action: "unequip-all" | "one-click") => void;
  onDone: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const equippedCountByCharacter = useMemo(() => {
    const counts = new Map<string, number>();
    for (const item of items) {
      if (!item.equippedCharacterUid) continue;
      const key = itemUidKey(item.equippedCharacterUid);
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    return counts;
  }, [items]);
  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog && !dialog.open) dialog.showModal();
    return () => {
      if (dialog?.open) dialog.close();
    };
  }, []);
  return (
    <dialog
      ref={dialogRef}
      aria-labelledby="empty-curtain-character-equipment-title"
      className="m-auto max-h-[calc(100dvh-3rem)] w-[min(1120px,calc(100vw-2rem))] overflow-hidden rounded-xl border bg-background p-0 text-foreground shadow-2xl outline-none backdrop:bg-background/60 backdrop:backdrop-blur-sm"
      onCancel={(event) => {
        event.preventDefault();
        onDone();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onDone();
      }}
    >
      <div className="flex max-h-[calc(100dvh-3rem)] min-h-0 flex-col">
        <header className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b bg-background/95 px-4 py-3 backdrop-blur">
          <div>
            <h2
              id="empty-curtain-character-equipment-title"
              className="text-base font-semibold"
            >
              {t("Character Equipment")}
            </h2>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {tf("{} characters", [String(characters.length)])}
            </p>
          </div>
          <div className="flex items-center gap-1.5">
            <Button
              variant="outline"
              size="sm"
              disabled={pending}
              onClick={onImport}
            >
              <Upload aria-hidden="true" />
              {t("Import Loadout")}
            </Button>
            <Button size="sm" onClick={onDone}>
              <Check aria-hidden="true" />
              {t("Done")}
            </Button>
          </div>
        </header>
        <div className="min-h-0 overflow-y-auto p-4">
          <div className="grid gap-2 min-[760px]:grid-cols-2 min-[1280px]:grid-cols-3">
            {characters.map((character) => {
              const equipped =
                equippedCountByCharacter.get(itemUidKey(character.uid)) ?? 0;
              return (
                <div
                  key={itemUidKey(character.uid)}
                  className="flex min-w-0 items-center gap-3 rounded-lg border bg-muted/20 p-2.5"
                >
                  <CharacterAvatar characterId={character.characterId} />
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">
                      {character.name}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {tf("{} equipped items", [String(equipped)])}
                    </p>
                  </div>
                  <div className="flex flex-wrap justify-end gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={pending}
                      onClick={() => onAction(character.uid, "one-click")}
                    >
                      {t("One-click Equip")}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={pending || equipped === 0}
                      onClick={() => onAction(character.uid, "unequip-all")}
                    >
                      {t("Unequip All")}
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t("Export Loadout")}
                      disabled={pending || equipped === 0}
                      onClick={() => onExport(character.uid)}
                    >
                      <Download aria-hidden="true" />
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </dialog>
  );
}

function FilterDialog({
  items,
  characters,
  filters,
  onChange,
  onDone,
}: {
  items: EmptyCurtainItem[];
  characters: EmptyCurtainCharacter[];
  filters: EquipmentFilters;
  onChange: (filters: EquipmentFilters) => void;
  onDone: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog && !dialog.open) dialog.showModal();
    return () => {
      if (dialog?.open) dialog.close();
    };
  }, []);

  return (
    <dialog
      ref={dialogRef}
      aria-labelledby="empty-curtain-filter-title"
      className="m-auto max-h-[calc(100dvh-3rem)] w-[min(1120px,calc(100vw-2rem))] overflow-hidden rounded-xl border bg-background p-0 text-foreground shadow-2xl outline-none backdrop:bg-background/60 backdrop:backdrop-blur-sm"
      onCancel={(event) => {
        event.preventDefault();
        onDone();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onDone();
      }}
    >
      <div className="flex max-h-[calc(100dvh-3rem)] min-h-0 flex-col">
        <header className="flex shrink-0 items-center justify-between gap-3 border-b bg-background/95 px-4 py-3 backdrop-blur">
          <div>
            <h2
              id="empty-curtain-filter-title"
              className="text-base font-semibold"
            >
              {t("Filter Console Equipment")}
            </h2>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {activeEquipmentFilterCount(filters) > 0
                ? tf("{} filters active", [
                    String(activeEquipmentFilterCount(filters)),
                  ])
                : t("Choose one or more conditions")}
            </p>
          </div>
          <div className="flex items-center gap-1.5">
            {activeEquipmentFilterCount(filters) > 0 ? (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => onChange(emptyEquipmentFilters())}
              >
                <RotateCcw aria-hidden="true" />
                {t("Clear Filters")}
              </Button>
            ) : null}
            <Button size="sm" onClick={onDone}>
              <Check aria-hidden="true" />
              {t("Done")}
            </Button>
          </div>
        </header>
        <div className="min-h-0 overflow-y-auto p-4">
          <FilterPanel
            items={items}
            characters={characters}
            filters={filters}
            onChange={onChange}
          />
        </div>
      </div>
    </dialog>
  );
}

function FilterPanel({
  items,
  characters,
  filters,
  onChange,
}: {
  items: EmptyCurtainItem[];
  characters: EmptyCurtainCharacter[];
  filters: EquipmentFilters;
  onChange: (filters: EquipmentFilters) => void;
}) {
  const [characterQuery, setCharacterQuery] = useState("");
  const [cassetteQuery, setCassetteQuery] = useState("");
  const { characterOptions, modules, cassettes, mainstats, substats } =
    useMemo(() => {
      const equippedCharacterIds = new Set(
        items.flatMap((item) =>
          item.equippedCharacterId === null ? [] : [item.equippedCharacterId],
        ),
      );
      return {
        characterOptions: uniqueBy(
          characters.filter((character) =>
            equippedCharacterIds.has(character.characterId),
          ),
          (character) => String(character.characterId),
        ).sort((left, right) => left.name.localeCompare(right.name)),
        modules: uniqueBy(
          items.filter((item) => item.kind === "module"),
          (item) => item.filterId,
        ).sort((left, right) => left.name.localeCompare(right.name)),
        cassettes: uniqueBy(
          items.filter((item) => item.kind === "core"),
          (item) => item.filterId,
        ).sort((left, right) => left.name.localeCompare(right.name)),
        mainstats: uniqueBy(
          items.flatMap((item) => item.stats.filter((stat) => stat.main)),
          (stat) => stat.property,
        ).sort((left, right) => left.label.localeCompare(right.label)),
        substats: uniqueBy(
          items.flatMap((item) => item.stats.filter((stat) => !stat.main)),
          (stat) => stat.property,
        ).sort((left, right) => left.label.localeCompare(right.label)),
      };
    }, [characters, items]);
  const visibleCharacters = characterOptions.filter((character) =>
    matchesEquipmentSearch(character.name, characterQuery),
  );
  const visibleCassettes = cassettes.filter((item) =>
    matchesEquipmentSearch(item.name, cassetteQuery),
  );
  return (
    <div className="grid gap-x-6 gap-y-5 min-[760px]:grid-cols-2 min-[1280px]:grid-cols-3">
      <FilterGroup
        label="By Character"
        controls={
          <FilterSearch
            value={characterQuery}
            onChange={setCharacterQuery}
            label="Search characters"
          />
        }
        contentClassName="max-h-48 overflow-y-auto overscroll-contain pr-1"
      >
        {visibleCharacters.map((character) => (
          <FilterChoice
            key={character.characterId}
            active={filters.characterIds.includes(character.characterId)}
            onClick={() =>
              onChange({
                ...filters,
                characterIds: toggle(
                  filters.characterIds,
                  character.characterId,
                ),
              })
            }
          >
            <CharacterAvatar characterId={character.characterId} small />
            <span className="max-w-40 truncate">{character.name}</span>
          </FilterChoice>
        ))}
        {visibleCharacters.length === 0 ? (
          <FilterNoMatches label="No matching characters" />
        ) : null}
      </FilterGroup>
      <EquipmentFilterGroup
        label="Drive Modules"
        items={modules}
        selected={filters.filterIds}
        contentClassName="max-h-48 overflow-y-auto overscroll-contain pr-1"
        onToggle={(id) =>
          onChange({ ...filters, filterIds: toggle(filters.filterIds, id) })
        }
      />
      <EquipmentFilterGroup
        label="Cassettes"
        items={visibleCassettes}
        selected={filters.filterIds}
        controls={
          <FilterSearch
            value={cassetteQuery}
            onChange={setCassetteQuery}
            label="Search cassettes"
          />
        }
        emptyLabel="No matching cassettes"
        contentClassName="max-h-48 overflow-y-auto overscroll-contain pr-1"
        onToggle={(id) =>
          onChange({ ...filters, filterIds: toggle(filters.filterIds, id) })
        }
      />
      <FilterGroup label="Quality">
        {QUALITY_OPTIONS.map((quality) => (
          <FilterChoice
            key={quality.id}
            active={filters.qualities.includes(quality.id)}
            onClick={() =>
              onChange({
                ...filters,
                qualities: toggle(filters.qualities, quality.id),
              })
            }
          >
            <span className={cn("size-2 rounded-full", quality.dotClassName)} />
            {t(quality.label)}
          </FilterChoice>
        ))}
      </FilterGroup>
      <FilterGroup
        label="Main stats"
        contentClassName="max-h-48 overflow-y-auto overscroll-contain pr-1"
      >
        {mainstats.map((stat) => (
          <FilterChoice
            key={stat.property}
            active={filters.mainstats.includes(stat.property)}
            onClick={() =>
              onChange({
                ...filters,
                mainstats: toggle(filters.mainstats, stat.property),
              })
            }
          >
            {stat.label}
          </FilterChoice>
        ))}
      </FilterGroup>
      <FilterGroup
        label="Substats (all selected properties must be present)"
        contentClassName="max-h-48 overflow-y-auto overscroll-contain pr-1"
      >
        {substats.map((stat) => (
          <FilterChoice
            key={stat.property}
            active={filters.substats.includes(stat.property)}
            onClick={() =>
              onChange({
                ...filters,
                substats: toggle(filters.substats, stat.property),
              })
            }
          >
            {stat.label}
          </FilterChoice>
        ))}
      </FilterGroup>
    </div>
  );
}

function EquipmentFilterGroup({
  label,
  items,
  selected,
  controls,
  emptyLabel,
  contentClassName,
  onToggle,
}: {
  label: string;
  items: EmptyCurtainItem[];
  selected: string[];
  controls?: React.ReactNode;
  emptyLabel?: string;
  contentClassName?: string;
  onToggle: (id: string) => void;
}) {
  return (
    <FilterGroup
      label={label}
      controls={controls}
      contentClassName={contentClassName}
    >
      {items.map((item) => (
        <FilterChoice
          key={item.filterId}
          active={selected.includes(item.filterId)}
          onClick={() => onToggle(item.filterId)}
        >
          <EquipmentImage item={item} className="size-6 rounded" />
          <span className="max-w-40 truncate">{item.name}</span>
        </FilterChoice>
      ))}
      {items.length === 0 && emptyLabel ? (
        <FilterNoMatches label={emptyLabel} />
      ) : null}
    </FilterGroup>
  );
}

function FilterGroup({
  label,
  controls,
  contentClassName,
  children,
}: {
  label: string;
  controls?: React.ReactNode;
  contentClassName?: string;
  children: React.ReactNode;
}) {
  return (
    <fieldset className="min-w-0">
      <legend className="mb-2 text-xs font-medium text-muted-foreground">
        {t(label)}
      </legend>
      {controls}
      <div className={cn("flex flex-wrap gap-1.5", contentClassName)}>
        {children}
      </div>
    </fieldset>
  );
}

function FilterSearch({
  value,
  onChange,
  label,
}: {
  value: string;
  onChange: (value: string) => void;
  label: string;
}) {
  return (
    <label className="relative mb-2 block">
      <Search
        className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
        aria-hidden="true"
      />
      <input
        type="search"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={t(label)}
        aria-label={t(label)}
        className="h-8 w-full rounded-md border bg-background pr-2 pl-8 text-xs outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
      />
    </label>
  );
}

function FilterNoMatches({ label }: { label: string }) {
  return (
    <p className="w-full py-2 text-xs text-muted-foreground" role="status">
      {t(label)}
    </p>
  );
}

function FilterChoice({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "inline-flex h-8 min-w-0 items-center gap-1.5 rounded-md border px-2.5 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
        active
          ? "border-primary bg-primary text-primary-foreground hover:bg-primary"
          : "border-border bg-background hover:bg-muted",
      )}
    >
      {children}
    </button>
  );
}

function EquipmentDetailsDialog({
  item,
  characters,
  pending,
  loadPositions,
  onManage,
  onDone,
}: {
  item: EmptyCurtainItem;
  characters: EmptyCurtainCharacter[];
  pending: boolean;
  loadPositions: (
    item: ItemUid,
    character: ItemUid,
  ) => Promise<EmptyCurtainPlacement[]>;
  onManage: (input: ManageItemInput) => void;
  onDone: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [targetKey, setTargetKey] = useState("");
  const [positions, setPositions] = useState<EmptyCurtainPlacement[]>([]);
  const [positionKey, setPositionKey] = useState("");
  const target = characters.find(
    (character) => itemUidKey(character.uid) === targetKey,
  );
  const equippedCharacter = characters.find(
    (character) => character.characterId === item.equippedCharacterId,
  );
  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog && !dialog.open) dialog.showModal();
    return () => {
      if (dialog?.open) dialog.close();
    };
  }, []);
  const selectCharacter = (key: string) => {
    setTargetKey(key);
    setPositions([]);
    setPositionKey("");
    const character = characters.find(
      (candidate) => itemUidKey(candidate.uid) === key,
    );
    if (character && item.kind === "module") {
      void loadPositions(item.uid, character.uid).then((value) => {
        setPositions(value);
        if (value[0]) setPositionKey(`${value[0].row}:${value[0].column}`);
      });
    }
  };
  const equip = () => {
    if (!target) return;
    const position = positions.find(
      (value) => `${value.row}:${value.column}` === positionKey,
    );
    onManage({
      item: item.uid,
      action: "equip",
      character: target.uid,
      position,
    });
  };
  return (
    <dialog
      ref={dialogRef}
      aria-labelledby="empty-curtain-item-title"
      className="m-auto max-h-[calc(100dvh-3rem)] w-[min(34rem,calc(100vw-2rem))] overflow-hidden rounded-xl border bg-background p-0 text-foreground shadow-2xl outline-none backdrop:bg-background/60 backdrop:backdrop-blur-sm"
      onCancel={(event) => {
        event.preventDefault();
        onDone();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onDone();
      }}
    >
      <article className="flex max-h-[calc(100dvh-3rem)] min-h-0 flex-col">
        <header className="flex shrink-0 items-center gap-3 border-b bg-muted/35 px-4 py-3">
          <EquipmentImage item={item} className="size-14 rounded-lg" />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5">
              <h2
                id="empty-curtain-item-title"
                className="min-w-0 flex-1 truncate text-base font-semibold"
              >
                {item.name}
              </h2>
              {item.locked ? (
                <LockKeyhole
                  className="size-4 shrink-0 text-muted-foreground"
                  aria-label={t("Lock")}
                />
              ) : null}
              {item.discarded ? (
                <Trash2
                  className="size-4 shrink-0 text-destructive"
                  aria-label={t("Discarded")}
                />
              ) : null}
            </div>
            <div className="mt-1 flex min-w-0 items-center gap-1.5">
              <span className="font-mono text-xs text-muted-foreground">
                {item.maxLevel
                  ? tf("Lv.{}/{}", [String(item.level), String(item.maxLevel)])
                  : tf("Lv.{}", [String(item.level)])}
              </span>
              {item.setName ? <SetBadge item={item} /> : null}
            </div>
          </div>
          {item.equippedCharacterId !== null ? (
            <Tooltip>
              <TooltipTrigger
                render={
                  <span
                    className="relative shrink-0 rounded-full ring-2 ring-background"
                    tabIndex={0}
                  >
                    <CharacterAvatar characterId={item.equippedCharacterId} />
                    <span className="absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full border-2 border-card bg-emerald-500" />
                  </span>
                }
              />
              <TooltipContent>
                {t("Equipped")} ·{" "}
                {equippedCharacter?.name ?? t("Unknown Character")}
              </TooltipContent>
            </Tooltip>
          ) : null}
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("Close")}
            onClick={onDone}
          >
            <X aria-hidden="true" />
          </Button>
        </header>
        <div className="min-h-0 overflow-y-auto">
          <dl className="divide-y divide-border/55 px-4">
            {item.stats
              .filter((stat) => stat.main)
              .map((stat, index) => (
                <div
                  key={`${stat.property}-${index}`}
                  className="flex min-w-0 items-center justify-between gap-3 py-2.5 text-sm"
                >
                  <dt className="truncate font-medium text-foreground">
                    {stat.label}
                  </dt>
                  <dd className="shrink-0 font-mono font-medium">
                    {formatStat(stat.value, stat.percent)}
                  </dd>
                </div>
              ))}
          </dl>
          {item.stats.some((stat) => !stat.main) ? (
            <section className="border-t px-4 py-3">
              <p className="mb-2 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
                {t("Secondary Stats")}
              </p>
              <div className="space-y-1">
                {item.stats
                  .filter((stat) => !stat.main)
                  .map((stat, index) => (
                    <div
                      key={`${stat.property}-${index}`}
                      className={cn(
                        "flex min-w-0 items-center gap-2 px-2 py-1.5 text-xs",
                        !stat.unlocked &&
                          "rounded-lg border border-dashed bg-muted/45 text-muted-foreground",
                      )}
                    >
                      <span className="min-w-0 flex-1 truncate">
                        {stat.label}
                      </span>
                      <span
                        className={cn(
                          "shrink-0 font-mono font-medium",
                          !stat.unlocked && "opacity-60",
                        )}
                      >
                        {formatStat(stat.value, stat.percent)}
                      </span>
                      {!stat.unlocked ? (
                        <>
                          <span
                            className="flex size-5 shrink-0 items-center justify-center rounded-full border bg-background/70"
                            aria-label={t("Locked")}
                          >
                            <LockKeyhole
                              className="size-3"
                              aria-hidden="true"
                            />
                          </span>
                          {stat.unlockLevel !== null ? (
                            <Badge
                              variant="outline"
                              className="h-5 min-w-9 justify-center px-1.5 font-mono text-[10px]"
                              title={tf("Unlocks at level {}", [
                                String(stat.unlockLevel),
                              ])}
                            >
                              +{stat.unlockLevel}
                            </Badge>
                          ) : null}
                        </>
                      ) : null}
                    </div>
                  ))}
              </div>
            </section>
          ) : null}
          <section className="space-y-3 border-t bg-muted/20 px-4 py-3">
            <div className="flex flex-wrap gap-1.5">
              <Button
                variant="outline"
                size="sm"
                disabled={pending}
                onClick={() =>
                  onManage({
                    item: item.uid,
                    action: item.locked ? "unlock" : "lock",
                  })
                }
              >
                {t(item.locked ? "Unlock" : "Lock")}
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={pending}
                onClick={() =>
                  onManage({
                    item: item.uid,
                    action: item.discarded ? "restore" : "discard",
                  })
                }
              >
                {t(item.discarded ? "Restore" : "Discard")}
              </Button>
              {item.equippedCharacterUid ? (
                <Button
                  variant="outline"
                  size="sm"
                  disabled={pending}
                  onClick={() =>
                    onManage({ item: item.uid, action: "unequip" })
                  }
                >
                  {t("Unequip")}
                </Button>
              ) : null}
            </div>
            {characters.length > 0 && item.kind ? (
              <div className="grid gap-2 min-[480px]:grid-cols-[minmax(0,1fr)_auto_auto]">
                <select
                  className="h-8 min-w-0 rounded-md border bg-background px-2 text-xs"
                  value={targetKey}
                  onChange={(event) => selectCharacter(event.target.value)}
                >
                  <option value="">{t("Choose Character")}</option>
                  {characters.map((character) => (
                    <option
                      key={itemUidKey(character.uid)}
                      value={itemUidKey(character.uid)}
                    >
                      {character.name}
                    </option>
                  ))}
                </select>
                {item.kind === "module" && target ? (
                  <select
                    className="h-8 min-w-24 rounded-md border bg-background px-2 text-xs"
                    value={positionKey}
                    onChange={(event) => setPositionKey(event.target.value)}
                  >
                    {positions.map((position) => (
                      <option
                        key={`${position.row}:${position.column}`}
                        value={`${position.row}:${position.column}`}
                      >
                        {position.row}, {position.column}
                      </option>
                    ))}
                  </select>
                ) : null}
                <Button
                  size="sm"
                  disabled={
                    pending ||
                    !target ||
                    (item.kind === "module" && !positionKey)
                  }
                  onClick={equip}
                >
                  {t("Equip")}
                </Button>
              </div>
            ) : null}
          </section>
        </div>
      </article>
    </dialog>
  );
}

function SetBadge({ item }: { item: EmptyCurtainItem }) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <span
            className="max-w-28 truncate rounded-full border bg-background/80 px-1.5 py-0.5 text-[10px] text-muted-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
            tabIndex={0}
          >
            {item.setName}
          </span>
        }
      />
      <TooltipContent className="block max-w-72 space-y-1.5 py-2">
        <p className="font-medium">{item.setName}</p>
        {item.setEffects.map((effect) => (
          <p key={effect.count} className="text-background/80">
            {tf("{}-Piece Set", [String(effect.count)])}: {effect.text}
          </p>
        ))}
      </TooltipContent>
    </Tooltip>
  );
}

function EquipmentImage({
  item,
  className,
}: {
  item: EmptyCurtainItem;
  className?: string;
}) {
  const imageUrl = equipmentImageUrl(item);
  return imageUrl ? (
    <img
      src={imageUrl}
      alt=""
      className={cn("shrink-0 object-cover", className)}
      draggable={false}
    />
  ) : (
    <span
      className={cn(
        "flex shrink-0 items-center justify-center bg-muted text-muted-foreground",
        className,
      )}
    >
      <Backpack className="size-1/2" aria-hidden="true" />
    </span>
  );
}

function equipmentImageUrl(item: EmptyCurtainItem): string | null {
  return item.icon ? (imageByFileName.get(fileName(item.icon)) ?? null) : null;
}

function CharacterAvatar({
  characterId,
  small = false,
}: {
  characterId: number;
  small?: boolean;
}) {
  const avatarUrl = characterAvatarUrl(characterId);
  const className = small ? "size-5" : "size-9";
  return avatarUrl ? (
    <img
      src={avatarUrl}
      alt=""
      className={cn(className, "rounded-full object-cover")}
      draggable={false}
    />
  ) : (
    <span
      className={cn(
        className,
        "flex items-center justify-center rounded-full bg-muted",
      )}
    >
      <UsersRound className="size-1/2" aria-hidden="true" />
    </span>
  );
}

function FloatingNotice({
  error,
  onClose,
}: {
  error: EmptyCurtainCommandError;
  onClose: () => void;
}) {
  return createPortal(
    <div className="pointer-events-none fixed inset-x-0 top-3 z-[100] flex justify-center px-3">
      <Alert
        variant="destructive"
        className="pointer-events-auto w-full max-w-xl bg-background shadow-lg"
      >
        <AlertTitle>{t("Console equipment operation failed")}</AlertTitle>
        <AlertDescription>
          {tf(error.messageKey, error.messageArguments)}
        </AlertDescription>
        <Button
          variant="ghost"
          size="icon-sm"
          className="absolute top-2 right-2"
          aria-label={t("Close")}
          onClick={onClose}
        >
          <X aria-hidden="true" />
        </Button>
      </Alert>
    </div>,
    document.body,
  );
}

function LoadingPage() {
  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
      <header className="border-b px-5 py-3">
        <Skeleton className="h-5 w-28" />
        <Skeleton className="mt-2 h-3 w-64" />
      </header>
      <div className="grid gap-3 p-4 min-[760px]:grid-cols-2 min-[1280px]:grid-cols-4">
        {Array.from({ length: 8 }, (_, index) => (
          <Skeleton key={index} className="h-48 rounded-xl" />
        ))}
      </div>
    </section>
  );
}

function LoadError({
  error,
  onRetry,
}: {
  error: EmptyCurtainCommandError;
  onRetry: () => void;
}) {
  return (
    <section className="flex min-h-0 min-w-0 flex-1 items-center justify-center p-5">
      <Alert variant="destructive" className="max-w-xl">
        <AlertTitle>{t("Failed to load Console equipment")}</AlertTitle>
        <AlertDescription>
          {tf(error.messageKey, error.messageArguments)}
        </AlertDescription>
        <Button className="mt-3" variant="outline" size="sm" onClick={onRetry}>
          {t("Retry")}
        </Button>
      </Alert>
    </section>
  );
}

function formatStat(value: number, percent: boolean): string {
  const scaled = value * (percent ? 100 : 1);
  const formatted = scaled
    .toFixed(2)
    .replace(/\.00$/, "")
    .replace(/(\.\d)0$/, "$1");
  return `+${formatted}${percent ? "%" : ""}`;
}
function toggle<T>(values: T[], value: T): T[] {
  return values.includes(value)
    ? values.filter((candidate) => candidate !== value)
    : [...values, value];
}
function uniqueBy<T>(values: T[], key: (value: T) => string): T[] {
  const seen = new Set<string>();
  return values.filter((value) => {
    const current = key(value);
    if (seen.has(current)) return false;
    seen.add(current);
    return true;
  });
}
function fileName(path: string): string {
  return path
    .slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1)
    .toLowerCase();
}
