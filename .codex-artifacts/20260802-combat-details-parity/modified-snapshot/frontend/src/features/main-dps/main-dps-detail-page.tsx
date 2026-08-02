import { ChevronDown, RefreshCw, SlidersHorizontal, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { DesktopTitlebar } from "@/components/nte/desktop-titlebar";
import { Button } from "@/components/ui/button";
import { characterAvatarUrl } from "@/lib/character-avatar";
import { t, tf, useTranslationRevision } from "@/lib/i18n";
import { useSettingsPresentation } from "@/lib/settings-presentation";
import { mainDpsDetailClient } from "@/lib/tauri/main-dps-detail-client";
import type {
  MainDpsAttributionSummary,
  MainDpsDetailFilter,
  MainDpsDetailSnapshot,
  MainDpsFilterSummary,
  MainDpsHit,
} from "@/lib/tauri/main-dps-detail-contract";
import { cn } from "@/lib/utils";

import { formatDuration, formatMainMetric } from "./main-dps-model";

interface VisibleColumns {
  character: boolean;
  type: boolean;
  damage: boolean;
  target: boolean;
}

const DEFAULT_COLUMNS: VisibleColumns = {
  character: true,
  type: true,
  damage: true,
  target: true,
};

export function MainDpsDetailPage() {
  useTranslationRevision();
  useSettingsPresentation();
  const [snapshot, setSnapshot] = useState<MainDpsDetailSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [columnsOpen, setColumnsOpen] = useState(false);
  const [columns, setColumns] = useState(DEFAULT_COLUMNS);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      setSnapshot(await mainDpsDetailClient.getSnapshot());
    } catch (loadError) {
      setError(errorText(loadError));
    } finally {
      setPending(false);
    }
  }, []);

  const loadMore = useCallback(async () => {
    if (snapshot === null || pending) return;
    setPending(true);
    setError(null);
    try {
      const next = await mainDpsDetailClient.getSnapshot(snapshot.rows.length);
      setSnapshot((current) =>
        current !== null && sameView(current, next)
          ? { ...next, offset: 0, rows: [...current.rows, ...next.rows] }
          : next,
      );
    } catch (loadError) {
      setError(errorText(loadError));
    } finally {
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
        setSnapshot(
          await mainDpsDetailClient.setView(filter, qteType, skillFilter),
        );
      } catch (viewError) {
        setError(errorText(viewError));
      } finally {
        setPending(false);
      }
    },
    [pending],
  );

  useEffect(() => {
    void load();
    let unlisten: (() => void) | null = null;
    void mainDpsDetailClient
      .subscribeRequested(() => void load())
      .then((value) => {
        unlisten = value;
      });
    return () => unlisten?.();
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
                size="sm"
                variant="outline"
                onClick={() => setColumnsOpen((value) => !value)}
              >
                <SlidersHorizontal />
                {t("Column settings")}
              </Button>
              {columnsOpen && (
                <div className="absolute right-0 top-9 z-20 grid min-w-44 gap-2 rounded-xl border bg-popover p-3 shadow-lg">
                  {(Object.keys(columns) as (keyof VisibleColumns)[]).map(
                    (column) => (
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
                          checked={columns[column]}
                          onChange={(event) =>
                            setColumns((value) => ({
                              ...value,
                              [column]: event.target.checked,
                            }))
                          }
                        />
                        {columnLabel(column)}
                      </label>
                    ),
                  )}
                </div>
              )}
            </div>
            <HitTable
              snapshot={snapshot}
              columns={columns}
              pending={pending}
              loadMore={loadMore}
            />
          </section>
        </main>
      )}
    </div>
  );
}

function DetailSummary({ snapshot }: { snapshot: MainDpsDetailSnapshot }) {
  const avatar =
    snapshot.characterId === null
      ? null
      : characterAvatarUrl(snapshot.characterId);
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
        {snapshot.skills.slice(0, 8).map((skill, index) => (
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
}: {
  snapshot: MainDpsDetailSnapshot;
  columns: VisibleColumns;
  pending: boolean;
  loadMore(): Promise<void>;
}) {
  if (snapshot.rows.length === 0)
    return (
      <div className="grid min-h-32 flex-1 place-items-center rounded-lg border border-dashed text-sm text-muted-foreground">
        {t("No hit records under the current filter")}
      </div>
    );

  return (
    <div className="min-h-0 flex-1 overflow-auto rounded-lg border bg-card">
      <table className="w-full min-w-[48rem] table-fixed text-sm">
        <thead className="sticky top-0 z-10 bg-background/95 text-left text-xs text-muted-foreground backdrop-blur">
          <tr>
            <th className="w-24 border-r px-2 py-2 font-medium">{t("Time")}</th>
            {snapshot.kind === "team" && columns.character && (
              <th className="w-36 border-r px-2 py-2 font-medium">
                {t("Character")}
              </th>
            )}
            {columns.type && (
              <th className="w-[34%] border-r px-2 py-2 font-medium">
                {t("Type")}
              </th>
            )}
            {columns.damage && (
              <th className="w-36 border-r px-2 py-2 font-medium">
                {t("Damage")}
              </th>
            )}
            {columns.target && (
              <th className="px-2 py-2 font-medium">{t("Target")} / HP</th>
            )}
          </tr>
        </thead>
        <tbody>
          {snapshot.rows.map((row) => (
            <HitRow
              key={row.id}
              row={row}
              team={snapshot.kind === "team"}
              columns={columns}
            />
          ))}
          {snapshot.rows.length < snapshot.totalHits && (
            <tr>
              <td colSpan={5} className="p-2 text-center">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={pending}
                  onClick={() => void loadMore()}
                >
                  {pending && <RefreshCw className="animate-spin" />}
                  {t("Load more")}
                </Button>
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

function HitRow({
  row,
  team,
  columns,
}: {
  row: MainDpsHit;
  team: boolean;
  columns: VisibleColumns;
}) {
  const avatar = characterAvatarUrl(row.characterId);
  const hpPercent =
    row.targetMaxHp > 0 ? Math.max(0, Math.min(100, row.targetHpPercent)) : 0;
  return (
    <tr className="border-t align-middle hover:bg-muted/35">
      <td className="border-r px-2 py-1.5 font-mono text-xs tabular-nums text-muted-foreground">
        {formatHitTime(row.timestamp)}
      </td>
      {team && columns.character && (
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
      {columns.type && (
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
            title={`${row.skill}\n${row.damageType}`}
          >
            {row.skill}
          </div>
        </td>
      )}
      {columns.damage && (
        <td
          className="border-r px-2 py-1.5 font-mono text-lg font-semibold tracking-[0.18em] tabular-nums text-cyan-950 [text-shadow:0_0_0.5px_#fff,0_0_1px_#0ff] dark:text-cyan-100"
          title={
            row.followUpDamage > 0
              ? tf("Damage: {} + {}", [
                  formatMainMetric(row.primaryDamage),
                  formatMainMetric(row.followUpDamage),
                ])
              : tf("Damage: {}", [formatMainMetric(row.damage)])
          }
        >
          {formatMainMetric(row.damage)}
        </td>
      )}
      {columns.target && (
        <td className="relative overflow-hidden px-2 py-1.5">
          {row.targetMaxHp > 0 && (
            <span
              className="absolute inset-y-1 left-1 rounded-md bg-emerald-600/10"
              style={{ width: `calc(${hpPercent}% - 0.5rem)` }}
            />
          )}
          <span className="relative block truncate">{row.target}</span>
          {row.targetMaxHp > 0 && (
            <span className="relative block truncate text-xs tabular-nums text-muted-foreground">
              {formatMainMetric(row.targetHpAfter)} /{" "}
              {formatMainMetric(row.targetMaxHp)} · {hpPercent.toFixed(1)}%
            </span>
          )}
        </td>
      )}
    </tr>
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

function columnLabel(column: keyof VisibleColumns): string {
  switch (column) {
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

function sameView(
  current: MainDpsDetailSnapshot,
  next: MainDpsDetailSnapshot,
): boolean {
  return (
    current.kind === next.kind &&
    current.characterId === next.characterId &&
    current.filter === next.filter &&
    current.qteType === next.qteType &&
    current.skillFilter === next.skillFilter
  );
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
