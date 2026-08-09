import { ChevronDown, RefreshCw, SlidersHorizontal, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { DesktopTitlebar } from "@/components/nte/desktop-titlebar";
import { Button } from "@/components/ui/button";
import { useCharacterAvatar } from "@/hooks/use-character-avatar";
import { useDismissibleLayer } from "@/hooks/use-dismissible-layer";
import { cleanupAsyncRegistration } from "@/lib/async-cleanup";
import {
  currentFrontendLanguage,
  t,
  tf,
  useTranslationRevision,
} from "@/lib/i18n";
import { useSettingsPresentation } from "@/lib/settings-presentation";
import { mainDpsDetailClient } from "@/lib/tauri/main-dps-detail-client";
import type {
  MainDpsAttributionSummary,
  MainDpsDetailColumns,
  MainDpsDetailFilter,
  MainDpsDetailSnapshot,
  MainDpsFilterSummary,
  MainDpsHit,
} from "@/lib/tauri/main-dps-detail-contract";
import { cn } from "@/lib/utils";
import { monsterImageUrl } from "@/features/abyss-values/abyss-values-model";

import { formatDuration, formatMainMetric } from "./main-dps-model";
import {
  DEFAULT_DETAIL_COLUMNS,
  DETAIL_ROW_HEIGHT,
  type DetailColumnKey,
  detailColumnVisible,
  detailColumnWidth,
  detailVisibleRowRange,
  mergeLiveDetailSnapshot,
  mergePagedDetailSnapshot,
  resetDetailColumnWidths,
  setDetailColumnVisible,
  setDetailColumnWidth,
} from "./main-dps-detail-model";

export function MainDpsDetailPage() {
  useTranslationRevision();
  useSettingsPresentation();
  const [snapshot, setSnapshot] = useState<MainDpsDetailSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [columnsOpen, setColumnsOpen] = useState(false);
  const columnsButtonRef = useRef<HTMLButtonElement>(null);
  const columnsMenuRef = useRef<HTMLDivElement>(null);
  const [columns, setColumns] = useState<MainDpsDetailColumns>(
    DEFAULT_DETAIL_COLUMNS,
  );
  const loadingMoreRef = useRef(false);
  useDismissibleLayer({
    open: columnsOpen,
    layerRef: columnsMenuRef,
    triggerRef: columnsButtonRef,
    onDismiss: () => setColumnsOpen(false),
  });
  const acceptSnapshot = useCallback((next: MainDpsDetailSnapshot) => {
    setSnapshot(next);
    setColumns(next.columns);
  }, []);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      acceptSnapshot(await mainDpsDetailClient.getSnapshot());
    } catch (loadError) {
      setError(errorText(loadError));
    } finally {
      setPending(false);
    }
  }, [acceptSnapshot]);

  const loadMore = useCallback(async () => {
    if (snapshot === null || pending || loadingMoreRef.current) return;
    loadingMoreRef.current = true;
    setPending(true);
    setError(null);
    try {
      const next = await mainDpsDetailClient.getSnapshot(snapshot.rows.length);
      setSnapshot((current) => mergePagedDetailSnapshot(current, next));
    } catch (loadError) {
      setError(errorText(loadError));
    } finally {
      loadingMoreRef.current = false;
      setPending(false);
    }
  }, [pending, snapshot]);

  const setView = useCallback(
    async (
      filter: MainDpsDetailFilter,
      qteType: string | null,
      skillFilter: string | null,
    ) => {
      if (pending) return;
      setPending(true);
      setError(null);
      try {
        acceptSnapshot(
          await mainDpsDetailClient.setView(filter, qteType, skillFilter),
        );
      } catch (viewError) {
        setError(errorText(viewError));
      } finally {
        setPending(false);
      }
    },
    [acceptSnapshot, pending],
  );

  const persistColumns = useCallback(
    (next: MainDpsDetailColumns) => {
      setColumns(next);
      void mainDpsDetailClient
        .setColumns(next)
        .then((saved) => setColumns(saved.columns))
        .catch((saveError) => {
          setError(errorText(saveError));
          void load();
        });
    },
    [load],
  );

  const runEmptyAction = useCallback(
    async (action: "start" | "import") => {
      setPending(true);
      setError(null);
      try {
        if (action === "start") await mainDpsDetailClient.startCapture();
        else await mainDpsDetailClient.importReplay();
        await load();
      } catch (actionError) {
        setError(errorText(actionError));
      } finally {
        setPending(false);
      }
    },
    [load],
  );

  useEffect(() => {
    void load();
    const cleanupRequested = cleanupAsyncRegistration(
      mainDpsDetailClient.subscribeRequested(() => void load()),
    );
    const unsubscribe = mainDpsDetailClient.subscribe(
      (next) => {
        setSnapshot((current) => {
          const merged = mergeLiveDetailSnapshot(current, next);
          return merged;
        });
        setColumns(next.columns);
      },
      (streamError) => setError(errorText(streamError)),
    );
    return () => {
      cleanupRequested();
      void unsubscribe();
    };
  }, [load]);

  const title = detailTitle(snapshot);

  return (
    <div className="desktop-window-shell">
      {error !== null && (
        <div className="fixed inset-x-0 top-3 z-50 mx-auto flex w-fit max-w-[min(92vw,36rem)] items-center gap-3 rounded-xl border border-destructive/30 bg-background/95 px-4 py-3 text-sm text-destructive shadow-lg backdrop-blur">
          <span>{error}</span>
          <button aria-label={t("Close")} onClick={() => setError(null)}>
            <X className="size-4" />
          </button>
        </div>
      )}
      <DesktopTitlebar
        title={title}
        onError={(value) => setError(errorText(value))}
      />

      {snapshot === null ? (
        <div className="grid min-h-0 flex-1 place-items-center">
          <div
            className="size-7 animate-spin rounded-full border-2 border-muted border-t-foreground"
            aria-label={t("Loading")}
          />
        </div>
      ) : (
        <main className="flex min-h-0 flex-1 flex-col gap-2 overflow-hidden p-3">
          <DetailSummary snapshot={snapshot} />

          <div className="flex min-w-0 flex-wrap items-center gap-2 py-0.5">
            <span className="text-sm font-medium text-muted-foreground">
              {t(snapshot.kind === "character" ? "Damage Type" : "Hit Type")}
            </span>
            {snapshot.hitTypes.map((summary) => (
              <FilterButton
                key={summary.id}
                active={snapshot.filter === summary.id}
                disabled={pending}
                onClick={() =>
                  void setView(summary.id, null, snapshot.skillFilter)
                }
              >
                {hitTypeLabel(summary)}
              </FilterButton>
            ))}
            {snapshot.kind === "character" && (
              <>
                <span className="mx-1 h-6 w-px bg-border" />
                <span className="text-sm font-medium text-muted-foreground">
                  {t("Specific Move")}
                </span>
                <label className="relative min-w-52 flex-1 sm:max-w-80">
                  <select
                    className="h-8 w-full appearance-none rounded-lg border bg-background px-3 pr-8 text-sm"
                    value={snapshot.skillFilter ?? ""}
                    disabled={pending}
                    onChange={(event) =>
                      void setView(
                        snapshot.filter,
                        snapshot.qteType,
                        event.target.value || null,
                      )
                    }
                  >
                    <option value="">{t("All moves")}</option>
                    {snapshot.skills.map((skill) => (
                      <option key={skill.id} value={skill.id}>
                        {skill.name} · {formatMainMetric(skill.damage)} ·{" "}
                        {skill.hits} {t("hits")}
                      </option>
                    ))}
                  </select>
                  <ChevronDown className="pointer-events-none absolute right-2 top-2 size-4" />
                </label>
              </>
            )}
            {pending && <RefreshCw className="ml-auto size-4 animate-spin" />}
          </div>

          {snapshot.kind === "team" && (
            <AttributionStrip
              snapshot={snapshot}
              pending={pending}
              setView={setView}
            />
          )}

          {snapshot.qteSummaries.length > 0 && (
            <div className="flex min-w-0 flex-wrap items-center gap-1.5">
              <span className="text-sm font-medium text-muted-foreground">
                {t("Reaction Damage")}
              </span>
              {snapshot.qteSummaries.map((summary) => (
                <FilterButton
                  key={summary.attackType}
                  active={
                    snapshot.filter === "qteType" &&
                    snapshot.qteType === summary.attackType
                  }
                  disabled={pending}
                  onClick={() =>
                    void setView(
                      "qteType",
                      summary.attackType,
                      snapshot.skillFilter,
                    )
                  }
                >
                  {summary.attackType} {formatMainMetric(summary.damage)} ·{" "}
                  {summary.sharePercent.toFixed(1)}%
                </FilterButton>
              ))}
            </div>
          )}

          {snapshot.kind === "character" && snapshot.skills.length > 0 && (
            <SkillBreakdown
              snapshot={snapshot}
              pending={pending}
              setView={setView}
            />
          )}

          <section className="flex min-h-0 flex-1 flex-col border-t pt-2">
            <div className="relative mb-1 flex items-center justify-between gap-3 text-xs text-muted-foreground">
              <span>{t("Drag column dividers to resize")}</span>
              <Button
                ref={columnsButtonRef}
                size="sm"
                variant="outline"
                aria-expanded={columnsOpen}
                aria-haspopup="menu"
                onClick={() => setColumnsOpen((value) => !value)}
              >
                <SlidersHorizontal />
                {t("Column settings")}
              </Button>
              {columnsOpen && (
                <div
                  ref={columnsMenuRef}
                  role="menu"
                  className="absolute right-0 top-9 z-20 grid min-w-44 gap-2 rounded-xl border bg-popover p-3 shadow-lg"
                >
                  {(
                    ["time", "character", "type", "damage", "target"] as const
                  ).map((column) => (
                    <label
                      key={column}
                      className={cn(
                        "flex items-center gap-2 text-sm",
                        snapshot.kind === "character" &&
                          column === "character" &&
                          "hidden",
                      )}
                    >
                      <input
                        type="checkbox"
                        checked={detailColumnVisible(columns, column)}
                        disabled={
                          detailColumnVisible(columns, column) &&
                          detailColumnsFor(snapshot, columns).length === 1
                        }
                        onChange={(event) =>
                          persistColumns(
                            setDetailColumnVisible(
                              columns,
                              column,
                              event.target.checked,
                            ),
                          )
                        }
                      />
                      {columnLabel(column)}
                    </label>
                  ))}
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() =>
                      persistColumns(resetDetailColumnWidths(columns))
                    }
                  >
                    {t("Reset column widths")}
                  </Button>
                </div>
              )}
            </div>
            <HitTable
              snapshot={snapshot}
              columns={columns}
              pending={pending}
              loadMore={loadMore}
              persistColumns={persistColumns}
              previewColumns={setColumns}
              clearFilters={() => void setView("all", null, null)}
              startCapture={() => void runEmptyAction("start")}
              importReplay={() => void runEmptyAction("import")}
            />
          </section>
        </main>
      )}
    </div>
  );
}

function DetailSummary({ snapshot }: { snapshot: MainDpsDetailSnapshot }) {
  const avatar = useCharacterAvatar(snapshot.characterId);
  const metrics = [
    ["Total Output", formatMainMetric(snapshot.metrics.totalOutput), false],
    ["DPS", formatMainMetric(snapshot.metrics.dps), false],
    ["Output Count", String(snapshot.metrics.outputCount), false],
    [
      "Total Damage Taken",
      formatMainMetric(snapshot.metrics.totalDamageTaken),
      true,
    ],
    ["Combat Time", formatDuration(snapshot.metrics.durationSeconds), false],
  ] as const;

  return (
    <section className="rounded-xl border bg-card p-2.5">
      <div className="flex min-w-0 gap-2.5">
        {snapshot.kind === "character" && (
          <div className="flex w-44 shrink-0 items-center gap-2 border-r pr-2.5 max-[780px]:w-36">
            <span
              className="grid size-14 shrink-0 place-items-center overflow-hidden rounded-xl bg-muted text-xl font-semibold"
              style={{ backgroundColor: snapshot.characterColor ?? undefined }}
            >
              {avatar ? (
                <img
                  src={avatar}
                  alt=""
                  className="size-full object-cover"
                  draggable={false}
                />
              ) : (
                snapshot.characterName?.slice(0, 1)
              )}
            </span>
            <span className="min-w-0">
              <strong className="block truncate text-base">
                {snapshot.characterName}
              </strong>
              <span className="block truncate text-xs text-muted-foreground">
                {tf("Character ID {}", [String(snapshot.characterId)])}
              </span>
            </span>
          </div>
        )}
        <div className="grid min-w-0 flex-1 grid-cols-5 divide-x max-[760px]:grid-cols-3 max-[540px]:grid-cols-2">
          {metrics.map(([label, value, danger]) => (
            <div key={label} className="min-w-0 px-2.5 py-1 text-center">
              <strong
                className={cn(
                  "block truncate font-mono text-lg font-medium tabular-nums",
                  danger && "text-destructive",
                )}
              >
                {value}
              </strong>
              <span className="block truncate text-xs text-muted-foreground">
                {t(label)}
              </span>
            </div>
          ))}
        </div>
      </div>
      <p className="mt-2 border-t pt-2 text-xs text-muted-foreground">
        {tf(
          "Confirmed output {} ({} hits) · candidate output {} ({} hits, {}% of total output)",
          [
            formatMainMetric(snapshot.direction.confirmedOutput),
            String(snapshot.direction.confirmedHits),
            formatMainMetric(snapshot.direction.candidateOutput),
            String(snapshot.direction.candidateHits),
            snapshot.direction.candidateSharePercent.toFixed(1),
          ],
        )}
      </p>
    </section>
  );
}

function AttributionStrip({
  snapshot,
  pending,
  setView,
}: {
  snapshot: MainDpsDetailSnapshot;
  pending: boolean;
  setView(
    filter: MainDpsDetailFilter,
    qteType: string | null,
    skillFilter: string | null,
  ): Promise<void>;
}) {
  const values: [
    MainDpsDetailFilter,
    string,
    keyof Pick<
      MainDpsAttributionSummary,
      | "characterDamage"
      | "reactionDamage"
      | "sharedDamage"
      | "unattributedDamage"
    >,
  ][] = [
    [
      snapshot.attribution.characterFilter,
      snapshot.attribution.separateReactionDamage
        ? "Character direct"
        : "Character attributed",
      "characterDamage",
    ],
    ["reactionDamage", "Reaction Damage", "reactionDamage"],
    ["sharedMechanics", "Shared mechanics", "sharedDamage"],
    ["unattributed", "Unattributed", "unattributedDamage"],
  ];
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1.5">
      <span className="text-sm font-medium text-muted-foreground">
        {t("Damage attribution")}
      </span>
      {values.map(([filter, label, field]) => (
        <FilterButton
          key={filter}
          active={snapshot.filter === filter}
          disabled={pending}
          onClick={() => void setView(filter, null, null)}
        >
          {t(label)}{" "}
          {share(snapshot.attribution[field], snapshot.attribution.totalDamage)}
        </FilterButton>
      ))}
    </div>
  );
}

function SkillBreakdown({
  snapshot,
  pending,
  setView,
}: {
  snapshot: MainDpsDetailSnapshot;
  pending: boolean;
  setView(
    filter: MainDpsDetailFilter,
    qteType: string | null,
    skillFilter: string | null,
  ): Promise<void>;
}) {
  return (
    <details open className="group rounded-lg border px-2.5 py-1.5">
      <summary className="cursor-pointer select-none text-sm font-medium">
        {t("Specific Move")}
      </summary>
      <div className="mt-1.5 grid gap-1 border-l pl-2">
        {snapshot.skills.map((skill, index) => (
          <button
            key={skill.id}
            type="button"
            disabled={pending}
            className={cn(
              "relative flex h-7 min-w-0 items-center justify-between overflow-hidden rounded-md px-2 text-left text-xs hover:bg-muted",
              snapshot.skillFilter === skill.id && "ring-1 ring-foreground",
            )}
            onClick={() =>
              void setView(
                snapshot.filter,
                snapshot.qteType,
                snapshot.skillFilter === skill.id ? null : skill.id,
              )
            }
          >
            <span
              className="absolute inset-y-0 left-0 bg-muted"
              style={{ width: `${Math.min(100, skill.sharePercent)}%` }}
            />
            <span className="relative min-w-0 truncate">
              {index + 1}. {skill.name}
            </span>
            <span className="relative ml-3 shrink-0 tabular-nums">
              {skill.sharePercent.toFixed(1)}% ·{" "}
              {formatMainMetric(skill.damage)} · {skill.hits}
              {t("hits")}
            </span>
          </button>
        ))}
      </div>
    </details>
  );
}

function HitTable({
  snapshot,
  columns,
  pending,
  loadMore,
  persistColumns,
  previewColumns,
  clearFilters,
  startCapture,
  importReplay,
}: {
  snapshot: MainDpsDetailSnapshot;
  columns: MainDpsDetailColumns;
  pending: boolean;
  loadMore(): Promise<void>;
  persistColumns(columns: MainDpsDetailColumns): void;
  previewColumns(columns: MainDpsDetailColumns): void;
  clearFilters(): void;
  startCapture(): void;
  importReplay(): void;
}) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(480);
  const hasRows = snapshot.rows.length > 0;
  const visibleColumns = detailColumnsFor(snapshot, columns);
  const colSpan = Math.max(1, visibleColumns.length);
  const totalWidth = visibleColumns.reduce(
    (total, column) => total + detailColumnWidth(columns, column),
    0,
  );

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const update = () => setViewportHeight(viewport.clientHeight);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [hasRows]);

  if (snapshot.rows.length === 0)
    return (
      <div className="grid min-h-32 flex-1 place-items-center rounded-lg border border-dashed p-5 text-sm text-muted-foreground">
        <div className="flex flex-wrap items-center justify-center gap-2">
          <span className="w-full text-center">
            {t("No hit records under the current filter")}
          </span>
          {(snapshot.filter !== "all" || snapshot.skillFilter !== null) && (
            <Button size="sm" variant="outline" onClick={clearFilters}>
              {t("Clear Filters")}
            </Button>
          )}
          <Button
            size="sm"
            variant="outline"
            disabled={pending || !snapshot.actions.canStartCapture}
            onClick={startCapture}
          >
            {t("Start")}
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={pending || !snapshot.actions.canImportReplay}
            onClick={importReplay}
          >
            {t("Import Capture JSON")}
          </Button>
        </div>
      </div>
    );

  const range = detailVisibleRowRange(
    snapshot.rows.length,
    scrollTop,
    viewportHeight,
  );
  const topSpacer = range.start * DETAIL_ROW_HEIGHT;
  const bottomSpacer = (snapshot.rows.length - range.end) * DETAIL_ROW_HEIGHT;

  return (
    <div
      ref={viewportRef}
      className="min-h-0 flex-1 overflow-auto rounded-lg border bg-card"
      onScroll={(event) => {
        const viewport = event.currentTarget;
        setScrollTop(viewport.scrollTop);
        if (
          snapshot.rows.length < snapshot.totalHits &&
          viewport.scrollTop + viewport.clientHeight >=
            viewport.scrollHeight - DETAIL_ROW_HEIGHT * 3
        ) {
          void loadMore();
        }
      }}
    >
      <table
        className="table-fixed w-full text-sm"
        style={{ minWidth: `${totalWidth}px` }}
      >
        <colgroup>
          {visibleColumns.map((column) => (
            <col
              key={column}
              style={{ width: `${detailColumnWidth(columns, column)}px` }}
            />
          ))}
        </colgroup>
        <thead className="sticky top-0 z-10 bg-background/95 text-left text-xs text-muted-foreground backdrop-blur">
          <tr>
            {visibleColumns.map((column) => (
              <ResizableHeader
                key={column}
                column={column}
                columns={columns}
                previewColumns={previewColumns}
                persistColumns={persistColumns}
              />
            ))}
          </tr>
        </thead>
        <tbody>
          {topSpacer > 0 && (
            <tr aria-hidden="true" style={{ height: `${topSpacer}px` }}>
              <td colSpan={colSpan} />
            </tr>
          )}
          {snapshot.rows.slice(range.start, range.end).map((row) => (
            <HitRow
              key={row.id}
              row={row}
              team={snapshot.kind === "team"}
              columns={columns}
              maxDamage={snapshot.maxRowDamage}
            />
          ))}
          {bottomSpacer > 0 && (
            <tr aria-hidden="true" style={{ height: `${bottomSpacer}px` }}>
              <td colSpan={colSpan} />
            </tr>
          )}
          {snapshot.rows.length < snapshot.totalHits && (
            <tr>
              <td colSpan={colSpan} className="p-2 text-center">
                <span className="inline-flex items-center gap-2 text-xs text-muted-foreground">
                  {pending && <RefreshCw className="size-4 animate-spin" />}
                  {t("Load more")}
                </span>
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

function ResizableHeader({
  column,
  columns,
  previewColumns,
  persistColumns,
}: {
  column: DetailColumnKey;
  columns: MainDpsDetailColumns;
  previewColumns(columns: MainDpsDetailColumns): void;
  persistColumns(columns: MainDpsDetailColumns): void;
}) {
  const beginResize = (event: React.PointerEvent<HTMLButtonElement>) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = detailColumnWidth(columns, column);
    let latest = columns;
    const move = (moveEvent: PointerEvent) => {
      latest = setDetailColumnWidth(
        columns,
        column,
        startWidth + moveEvent.clientX - startX,
      );
      previewColumns(latest);
    };
    const finish = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      persistColumns(latest);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish, { once: true });
  };
  return (
    <th className="relative border-r px-2 py-2 font-medium last:border-r-0">
      {columnLabel(column)}
      <button
        type="button"
        aria-label={`${t("Drag column dividers to resize")}: ${columnLabel(column)}`}
        className="absolute inset-y-0 -right-1 z-10 w-2 cursor-col-resize touch-none hover:bg-foreground/20"
        onPointerDown={beginResize}
        onDoubleClick={() =>
          persistColumns(
            setDetailColumnWidth(
              columns,
              column,
              detailColumnWidth(DEFAULT_DETAIL_COLUMNS, column),
            ),
          )
        }
      />
    </th>
  );
}

function HitRow({
  row,
  team,
  columns,
  maxDamage,
}: {
  row: MainDpsHit;
  team: boolean;
  columns: MainDpsDetailColumns;
  maxDamage: number;
}) {
  const avatar = useCharacterAvatar(row.characterId);
  const targetPortrait = row.targetMonsterId
    ? monsterImageUrl(row.targetMonsterId)
    : null;
  const hpPercent =
    row.targetMaxHp > 0 ? Math.max(0, Math.min(100, row.targetHpPercent)) : 0;
  const damagePercent = Math.max(
    0,
    Math.min(100, (row.damage / maxDamage) * 100),
  );
  return (
    <tr
      className="h-16 border-t align-middle hover:bg-muted/35"
      style={{
        backgroundImage: `linear-gradient(to right, color-mix(in srgb, var(--muted) 50%, transparent) ${damagePercent}%, transparent ${damagePercent}%)`,
      }}
    >
      {columns.showTime && (
        <td className="border-r px-2 py-1.5 font-mono text-xs tabular-nums text-muted-foreground">
          {formatHitTime(row.timestamp)}
        </td>
      )}
      {team && columns.showCharacter && (
        <td className="border-r px-2 py-1.5">
          <span className="flex min-w-0 items-center gap-2">
            <span className="grid size-7 shrink-0 place-items-center overflow-hidden rounded-md bg-muted text-xs">
              {avatar ? (
                <img src={avatar} alt="" className="size-full object-cover" />
              ) : (
                row.characterName.slice(0, 1)
              )}
            </span>
            <span className="truncate">{row.characterName}</span>
          </span>
        </td>
      )}
      {columns.showType && (
        <td className="border-r p-1.5">
          <div
            className={cn(
              "truncate rounded-lg px-3 py-2 text-center text-xs",
              row.direction === "outgoing"
                ? "bg-foreground text-background"
                : row.direction === "incoming"
                  ? "bg-destructive/10 text-destructive"
                  : "border border-dashed bg-muted text-muted-foreground",
            )}
            title={`${row.typeLabel}\n${row.skill}\n${row.damageType}`}
          >
            {row.reactionTextKey === null ? (
              row.typeLabel
            ) : (
              <ReactionLabelImages
                reaction={row.reactionTextKey}
                fallback={row.typeLabel}
              />
            )}
          </div>
        </td>
      )}
      {columns.showDamage && (
        <td
          className="border-r px-2 py-1.5"
          title={
            row.followUpDamage > 0
              ? tf("Damage: {} + {}", [
                  formatMainMetric(row.primaryDamage),
                  formatMainMetric(row.followUpDamage),
                ])
              : tf("Damage: {}", [formatMainMetric(row.damage)])
          }
        >
          <span className="relative inline-flex min-h-7 items-center">
            <DamageDigits
              value={row.primaryDamage}
              digitKey={row.damageDigitKey}
            />
            {row.followUpDamage > 0 && (
              <span className="ml-1 self-start rounded bg-background/80 px-1 py-0.5 shadow-sm">
                <DamageDigits
                  value={row.followUpDamage}
                  digitKey={row.followUpDamageDigitKey}
                  compact
                />
              </span>
            )}
          </span>
        </td>
      )}
      {columns.showTarget && (
        <td className="relative overflow-hidden px-2 py-1.5">
          {row.targetMaxHp > 0 && (
            <span
              className={cn(
                "absolute inset-y-1 left-1 rounded-md",
                hpPercent > 50
                  ? "bg-emerald-600/15"
                  : hpPercent > 20
                    ? "bg-amber-500/20"
                    : "bg-destructive/20",
              )}
              style={{ width: `calc(${hpPercent}% - 0.5rem)` }}
            />
          )}
          <span className="relative flex min-w-0 items-center gap-2">
            {targetPortrait && (
              <img
                src={targetPortrait}
                alt=""
                className="size-9 shrink-0 rounded-md object-cover"
                draggable={false}
              />
            )}
            <span className="min-w-0">
              <span className="block truncate">{row.target}</span>
              {row.targetMaxHp > 0 && (
                <span className="block truncate text-xs tabular-nums text-muted-foreground">
                  {formatMainMetric(row.targetHpAfter)} /{" "}
                  {formatMainMetric(row.targetMaxHp)} · {hpPercent.toFixed(1)}%
                </span>
              )}
            </span>
          </span>
        </td>
      )}
    </tr>
  );
}

const damageDigitImages = import.meta.glob<string>(
  "@res/images/font/tiaozi1/*.png",
  { eager: true, query: "?url", import: "default" },
);
const reactionLabelImages = import.meta.glob<string>(
  "@res/images/font/tiaozi1/{zh,en,ja}/fanying*.png",
  { eager: true, query: "?url", import: "default" },
);
const damageDigitPrefix: Record<string, string> = {
  灵: "ling",
  咒: "zhou",
  光: "guang",
  魂: "hun",
  暗: "an",
  相: "xiang",
  物理: "wuli",
  HP: "HP",
  真实: "zhenshi",
  Guangling_G: "Guangling_G",
  Guangxiang_G: "Guangxiang_G",
  Guangxiang_X: "Guangxiang_X",
  Hunxiang_H: "Hunxiang_H",
  Hunxiang_X: "Hunxiang_X",
  Anhun_A: "Anhun_A",
  Zhouan_A: "Zhouan_A",
  lingzhou_L: "lingzhou_L",
};

function DamageDigits({
  value,
  digitKey,
  compact = false,
}: {
  value: number;
  digitKey: string | null;
  compact?: boolean;
}) {
  const digits = Math.round(Math.max(0, value)).toString();
  const prefix = digitKey ? damageDigitPrefix[digitKey] : undefined;
  const urls = prefix
    ? [...digits].map(
        (digit) =>
          Object.entries(damageDigitImages).find(([path]) =>
            path.endsWith(`/${prefix}_${digit}.png`),
          )?.[1],
      )
    : [];
  if (urls.length !== digits.length || urls.some((url) => url === undefined)) {
    return (
      <span
        className={cn(
          "font-mono font-semibold tabular-nums text-cyan-950 dark:text-cyan-100",
          compact ? "text-xs" : "text-lg",
        )}
      >
        {formatMainMetric(value)}
      </span>
    );
  }
  return (
    <span
      className="inline-flex items-center"
      aria-label={formatMainMetric(value)}
    >
      {urls.map((url, index) => (
        <img
          key={`${index}:${digits[index]}`}
          src={url}
          alt=""
          className={cn("w-auto object-contain", compact ? "h-3.5" : "h-6")}
          draggable={false}
        />
      ))}
    </span>
  );
}

function ReactionLabelImages({
  reaction,
  fallback,
}: {
  reaction: number;
  fallback: string;
}) {
  const language = currentFrontendLanguage();
  const folder = language === "zh-CN" ? "zh" : language;
  const stem = `fanying${String(reaction).padStart(2, "0")}`;
  const urls = [1, 2]
    .map(
      (part) =>
        Object.entries(reactionLabelImages).find(([path]) =>
          path.endsWith(
            `/${folder}/${stem}_${String(part).padStart(2, "0")}.png`,
          ),
        )?.[1],
    )
    .filter((url): url is string => url !== undefined);
  if (urls.length === 0) return fallback;
  return (
    <span className="inline-flex h-5 items-center justify-center gap-0.5">
      {urls.map((url) => (
        <img key={url} src={url} alt="" className="h-5 w-auto object-contain" />
      ))}
    </span>
  );
}

function detailColumnsFor(
  snapshot: MainDpsDetailSnapshot,
  columns: MainDpsDetailColumns,
): DetailColumnKey[] {
  return (["time", "character", "type", "damage", "target"] as const).filter(
    (column) =>
      detailColumnVisible(columns, column) &&
      !(snapshot.kind === "character" && column === "character"),
  );
}

function FilterButton({
  active,
  disabled,
  onClick,
  children,
}: {
  active: boolean;
  disabled: boolean;
  onClick(): void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      className={cn(
        "h-8 rounded-lg border px-3 text-sm transition-colors disabled:opacity-50",
        active
          ? "border-foreground bg-foreground text-background"
          : "bg-background hover:bg-muted",
      )}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

function detailTitle(snapshot: MainDpsDetailSnapshot | null): string {
  if (snapshot?.kind === "character")
    return tf("{} - Combat Details", [
      snapshot.characterName ?? t("Character"),
    ]);
  if (snapshot?.abyssHalf)
    return tf("Team Combat Details - {}", [
      t(snapshot.abyssHalf === "first" ? "First Half" : "Second Half"),
    ]);
  return t("Team Combat Details");
}

function hitTypeLabel(summary: MainDpsFilterSummary): string {
  if (summary.id === "outgoing")
    return tf("Outgoing {}", [String(summary.hits)]);
  if (summary.id === "incoming") return tf("Taken {}", [String(summary.hits)]);
  return tf("All {}", [String(summary.hits)]);
}

function columnLabel(column: DetailColumnKey): string {
  switch (column) {
    case "time":
      return t("Time");
    case "character":
      return t("Character");
    case "type":
      return t("Type");
    case "damage":
      return t("Damage");
    case "target":
      return t("Target");
  }
}

function share(value: number, total: number): string {
  return `${total > 0 ? ((value / total) * 100).toFixed(1) : "0.0"}%`;
}

function formatHitTime(timestamp: number): string {
  if (timestamp > 86_400)
    return new Date(timestamp * 1_000).toLocaleTimeString([], {
      hour12: false,
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  return `${timestamp.toFixed(1)}s`;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
