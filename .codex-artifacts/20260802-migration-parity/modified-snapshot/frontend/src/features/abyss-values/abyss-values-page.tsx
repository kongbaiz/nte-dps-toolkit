import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
} from "react";
import {
  ChevronDown,
  ChevronRight,
  RefreshCw,
  Search,
  TriangleAlert,
  X,
} from "lucide-react";

import statDisplayNames from "@res/data/abyss/monster_stat_names_zh_cn.json";

import { DesktopTitlebar } from "@/components/nte/desktop-titlebar";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { t, tf } from "@/lib/i18n";
import { useSettingsPresentation } from "@/lib/settings-presentation";
import {
  abyssValuesClient,
  type AbyssValuesClient,
} from "@/lib/tauri/abyss-values-client";
import {
  abyssValuesError,
  type AbyssFloor,
  type AbyssHalfId,
  type AbyssMonster,
  type AbyssPredictionTeams,
  type AbyssTeam,
  type AbyssValuesCommandError,
  type AbyssValuesSnapshot,
} from "@/lib/tauri/abyss-values-contract";
import { cn } from "@/lib/utils";

import {
  filterMonsters,
  findFloor,
  formatSeconds,
  formatStatValue,
  initialFloorKey,
  lineTotalHp,
  monsterImageUrl,
  monstersForHalf,
  monsterTotalHp,
  predictedSeconds,
  requiredDps,
  summarizeWaves,
  type AbyssFloorKey,
} from "./abyss-values-model";

type PageState =
  | { status: "loading" }
  | { status: "error"; error: AbyssValuesCommandError }
  | { status: "ready"; snapshot: AbyssValuesSnapshot };

const STAT_DISPLAY_NAMES: Readonly<Record<string, string>> = statDisplayNames;

export function AbyssValuesPage({
  client = abyssValuesClient,
}: {
  client?: AbyssValuesClient;
}) {
  const presentation = useSettingsPresentation();
  const [state, setState] = useState<PageState>({ status: "loading" });
  const [selectedFloorKey, setSelectedFloorKey] =
    useState<AbyssFloorKey | null>(null);
  const [expandedSeason, setExpandedSeason] = useState<number | null>(null);
  const [selectedPackId, setSelectedPackId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState(false);
  const [mutationError, setMutationError] =
    useState<AbyssValuesCommandError | null>(null);

  const load = useCallback(async () => {
    setState({ status: "loading" });
    setMutationError(null);
    try {
      const snapshot = await client.getSnapshot();
      const initial = initialFloorKey(snapshot.seasons);
      setSelectedFloorKey((current) =>
        findFloor(snapshot.seasons, current) ? current : initial,
      );
      setExpandedSeason((current) => current ?? initial?.season ?? null);
      setSelectedPackId(null);
      setState({ status: "ready", snapshot });
    } catch (error) {
      setState({ status: "error", error: abyssValuesError(error) });
    }
  }, [client]);

  useEffect(() => {
    void load();
  }, [load]);

  const updateTeams = useCallback(
    async (operation: () => Promise<AbyssPredictionTeams>) => {
      if (state.status !== "ready" || pending) return;
      setPending(true);
      setMutationError(null);
      try {
        const teams = await operation();
        setState({
          status: "ready",
          snapshot: { ...state.snapshot, teams },
        });
      } catch (error) {
        setMutationError(abyssValuesError(error));
      } finally {
        setPending(false);
      }
    },
    [pending, state],
  );

  return (
    <div
      className={cn(
        "desktop-window-shell",
        !presentation.darkMode && "console-light",
      )}
    >
      <DesktopTitlebar title={t("Abyss monster values")} />
      <main className="flex min-h-0 flex-1 flex-col overflow-hidden bg-background text-foreground">
        {mutationError ? (
          <FloatingError
            error={mutationError}
            onDismiss={() => setMutationError(null)}
          />
        ) : null}
        {state.status === "loading" ? (
          <AbyssValuesLoading />
        ) : state.status === "error" ? (
          <AbyssValuesError error={state.error} onRetry={load} />
        ) : (
          <AbyssValuesReady
            snapshot={state.snapshot}
            selectedFloorKey={selectedFloorKey}
            expandedSeason={expandedSeason}
            selectedPackId={selectedPackId}
            query={query}
            pending={pending}
            onReload={load}
            onQueryChange={setQuery}
            onExpandSeason={setExpandedSeason}
            onSelectFloor={(key) => {
              setSelectedFloorKey(key);
              setExpandedSeason(key.season);
              setSelectedPackId(null);
            }}
            onSelectMonster={setSelectedPackId}
            onSwapTeams={() => updateTeams(() => client.swapTeams())}
            onClearTeam={(half) => updateTeams(() => client.clearTeam(half))}
            onImportTeam={(half, json) =>
              updateTeams(() => client.importTeam(half, json))
            }
            onImportCurrent={(half) =>
              updateTeams(() => client.importCurrentTeam(half))
            }
          />
        )}
      </main>
    </div>
  );
}

function AbyssValuesReady({
  snapshot,
  selectedFloorKey,
  expandedSeason,
  selectedPackId,
  query,
  pending,
  onReload,
  onQueryChange,
  onExpandSeason,
  onSelectFloor,
  onSelectMonster,
  onSwapTeams,
  onClearTeam,
  onImportTeam,
  onImportCurrent,
}: {
  snapshot: AbyssValuesSnapshot;
  selectedFloorKey: AbyssFloorKey | null;
  expandedSeason: number | null;
  selectedPackId: string | null;
  query: string;
  pending: boolean;
  onReload: () => Promise<void>;
  onQueryChange: (query: string) => void;
  onExpandSeason: (season: number | null) => void;
  onSelectFloor: (key: AbyssFloorKey) => void;
  onSelectMonster: (packId: string) => void;
  onSwapTeams: () => void;
  onClearTeam: (half: AbyssHalfId) => void;
  onImportTeam: (half: AbyssHalfId, json: string) => void;
  onImportCurrent: (half: AbyssHalfId) => void;
}) {
  const selectedFloor = findFloor(snapshot.seasons, selectedFloorKey);
  const filteredMonsters = useMemo(
    () => filterMonsters(selectedFloor?.monsters ?? [], query),
    [query, selectedFloor],
  );
  const selectedMonster =
    filteredMonsters.find((monster) => monster.packId === selectedPackId) ??
    filteredMonsters[0] ??
    null;

  return (
    <>
      <header className="flex h-14 shrink-0 items-center justify-between gap-3 border-b px-3">
        <p className="text-sm font-medium">
          {tf("{} seasons · {} floors · {} abyss enemies", [
            String(snapshot.seasonCount),
            String(snapshot.floorCount),
            String(snapshot.monsterCount),
          ])}
        </p>
        <Button
          variant="outline"
          disabled={pending}
          onClick={() => void onReload()}
        >
          <RefreshCw className="size-4" aria-hidden="true" />
          {t("Reload")}
        </Button>
      </header>
      <div className="flex min-h-0 flex-1 overflow-hidden">
        <AbyssNavigation
          snapshot={snapshot}
          selectedFloorKey={selectedFloorKey}
          expandedSeason={expandedSeason}
          onExpandSeason={onExpandSeason}
          onSelectFloor={onSelectFloor}
        />
        <section className="min-w-0 flex-1 overflow-y-auto p-2.5">
          {selectedFloor ? (
            <AbyssFloorContent
              floor={selectedFloor}
              monsters={filteredMonsters}
              selectedMonster={selectedMonster}
              query={query}
              teams={snapshot.teams}
              pending={pending}
              onQueryChange={onQueryChange}
              onSelectMonster={onSelectMonster}
              onSwapTeams={onSwapTeams}
              onClearTeam={onClearTeam}
              onImportTeam={onImportTeam}
              onImportCurrent={onImportCurrent}
            />
          ) : (
            <div className="grid h-full place-items-center text-sm text-muted-foreground">
              {t("Select an abyss site")}
            </div>
          )}
        </section>
      </div>
    </>
  );
}

function AbyssNavigation({
  snapshot,
  selectedFloorKey,
  expandedSeason,
  onExpandSeason,
  onSelectFloor,
}: {
  snapshot: AbyssValuesSnapshot;
  selectedFloorKey: AbyssFloorKey | null;
  expandedSeason: number | null;
  onExpandSeason: (season: number | null) => void;
  onSelectFloor: (key: AbyssFloorKey) => void;
}) {
  return (
    <aside className="flex w-[228px] shrink-0 flex-col border-r max-[840px]:w-[204px]">
      <h1 className="shrink-0 px-3 pt-3 pb-2 text-sm font-medium text-muted-foreground">
        {t("Sites")}
      </h1>
      <nav className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {snapshot.seasons.map((season) => {
          const expanded = season.season === expandedSeason;
          return (
            <div className="mb-1.5" key={season.season}>
              <button
                className={cn(
                  "flex h-8 w-full items-center gap-2 rounded-md border px-2 text-left text-sm transition-colors hover:bg-muted",
                  expanded &&
                    "bg-foreground text-background hover:bg-foreground/90",
                )}
                type="button"
                onClick={() => onExpandSeason(expanded ? null : season.season)}
              >
                {expanded ? (
                  <ChevronDown className="size-3.5 shrink-0" />
                ) : (
                  <ChevronRight className="size-3.5 shrink-0" />
                )}
                <span className="min-w-0 flex-1 truncate">
                  {season.name || tf("Season {}", [String(season.season)])}
                </span>
                <span className="shrink-0 text-xs opacity-70">
                  {tf("{} floors", [String(season.floors.length)])}
                </span>
              </button>
              {expanded ? (
                <div className="mt-1 ml-3 border-l pl-1.5">
                  {season.floors.map((floor) => {
                    const selected =
                      selectedFloorKey?.season === floor.season &&
                      selectedFloorKey.floor === floor.floor;
                    return (
                      <button
                        className={cn(
                          "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-xs transition-colors hover:bg-muted",
                          selected &&
                            "bg-foreground text-background hover:bg-foreground/90",
                        )}
                        key={floor.floor}
                        type="button"
                        onClick={() =>
                          onSelectFloor({
                            season: floor.season,
                            floor: floor.floor,
                          })
                        }
                      >
                        <span className="min-w-0 flex-1 truncate text-sm">
                          {floor.name || tf("Floor {}", [String(floor.floor)])}
                        </span>
                        <span className="shrink-0 opacity-70">
                          {tf("{} monsters · {} waves", [
                            String(floor.monsterCount),
                            String(floor.waveCount),
                          ])}
                        </span>
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </div>
          );
        })}
      </nav>
    </aside>
  );
}

function AbyssFloorContent({
  floor,
  monsters,
  selectedMonster,
  query,
  teams,
  pending,
  onQueryChange,
  onSelectMonster,
  onSwapTeams,
  onClearTeam,
  onImportTeam,
  onImportCurrent,
}: {
  floor: AbyssFloor;
  monsters: AbyssMonster[];
  selectedMonster: AbyssMonster | null;
  query: string;
  teams: AbyssPredictionTeams;
  pending: boolean;
  onQueryChange: (query: string) => void;
  onSelectMonster: (packId: string) => void;
  onSwapTeams: () => void;
  onClearTeam: (half: AbyssHalfId) => void;
  onImportTeam: (half: AbyssHalfId, json: string) => void;
  onImportCurrent: (half: AbyssHalfId) => void;
}) {
  const upper = monstersForHalf(monsters, 0);
  const lower = monstersForHalf(monsters, 1);
  const unassigned = monsters.filter(
    (monster) => monster.half !== 0 && monster.half !== 1,
  );
  const hasTeams = teams.upper !== null || teams.lower !== null;

  return (
    <div className="mx-auto flex w-full max-w-[1600px] flex-col gap-2.5">
      <div className="flex min-h-9 flex-wrap items-center justify-between gap-2">
        <h2 className="font-heading text-base font-medium">
          {floor.seasonName || tf("Season {}", [String(floor.season)])} ·{" "}
          {floor.name || tf("Floor {}", [String(floor.floor)])}
        </h2>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={pending || !hasTeams}
            onClick={onSwapTeams}
          >
            {t("Swap Teams")}
          </Button>
          <label className="relative w-[190px] max-[900px]:w-[160px]">
            <Search
              className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
            <input
              className="h-8 w-full rounded-md border bg-background pr-2 pl-8 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
              value={query}
              onChange={(event) => onQueryChange(event.target.value)}
              placeholder={t("Search monster / ID")}
              aria-label={t("Search monster / ID")}
            />
          </label>
        </div>
      </div>

      {monsters.length === 0 ? (
        <div className="grid h-28 place-items-center rounded-lg border text-sm text-muted-foreground">
          {t("No matching monster")}
        </div>
      ) : (
        <>
          {upper.length > 0 || lower.length > 0 ? (
            <>
              <AbyssLineSection
                half="upper"
                title={t("Ascending Line")}
                monsters={upper}
                team={teams.upper}
                selectedPackId={selectedMonster?.packId ?? null}
                recommendedElements={floor.recommendedElements.firstHalf}
                maxSeconds={floor.maxSeconds}
                thresholds={floor.starThresholds}
                pending={pending}
                onSelectMonster={onSelectMonster}
                onClearTeam={onClearTeam}
                onImportTeam={onImportTeam}
                onImportCurrent={onImportCurrent}
              />
              <AbyssLineSection
                half="lower"
                title={t("Descending Line")}
                monsters={lower}
                team={teams.lower}
                selectedPackId={selectedMonster?.packId ?? null}
                recommendedElements={floor.recommendedElements.secondHalf}
                maxSeconds={floor.maxSeconds}
                thresholds={floor.starThresholds}
                pending={pending}
                onSelectMonster={onSelectMonster}
                onClearTeam={onClearTeam}
                onImportTeam={onImportTeam}
                onImportCurrent={onImportCurrent}
              />
            </>
          ) : null}
          {unassigned.length > 0 ? (
            <AbyssLineSection
              half="upper"
              title={t("Full floor config")}
              monsters={unassigned}
              team={null}
              selectedPackId={selectedMonster?.packId ?? null}
              recommendedElements={[]}
              maxSeconds={floor.maxSeconds}
              thresholds={[]}
              pending={pending}
              predictionEnabled={false}
              onSelectMonster={onSelectMonster}
              onClearTeam={onClearTeam}
              onImportTeam={onImportTeam}
              onImportCurrent={onImportCurrent}
            />
          ) : null}
        </>
      )}

      <div className="border-t pt-2.5">
        {selectedMonster ? (
          <AbyssMonsterDetail monster={selectedMonster} />
        ) : (
          <div className="grid h-36 place-items-center text-sm text-muted-foreground">
            {t("Click a monster card to see all stat fields")}
          </div>
        )}
      </div>
    </div>
  );
}

function AbyssLineSection({
  half,
  title,
  monsters,
  team,
  selectedPackId,
  recommendedElements,
  maxSeconds,
  thresholds,
  pending,
  predictionEnabled = true,
  onSelectMonster,
  onClearTeam,
  onImportTeam,
  onImportCurrent,
}: {
  half: AbyssHalfId;
  title: string;
  monsters: AbyssMonster[];
  team: AbyssTeam | null;
  selectedPackId: string | null;
  recommendedElements: string[];
  maxSeconds: number | null;
  thresholds: AbyssFloor["starThresholds"];
  pending: boolean;
  predictionEnabled?: boolean;
  onSelectMonster: (packId: string) => void;
  onClearTeam: (half: AbyssHalfId) => void;
  onImportTeam: (half: AbyssHalfId, json: string) => void;
  onImportCurrent: (half: AbyssHalfId) => void;
}) {
  const defaultTarget =
    thresholds.reduce(
      (best, threshold) => (threshold.stars > best.stars ? threshold : best),
      { stars: 0, seconds: Math.min(maxSeconds ?? 90, 90) },
    ).seconds || 90;
  const [targetSeconds, setTargetSeconds] = useState(defaultTarget);
  const fileInput = useRef<HTMLInputElement>(null);
  const waves = summarizeWaves(monsters);
  const hp = lineTotalHp(monsters);
  const needDps = requiredDps(monsters, targetSeconds);
  const estimate = predictedSeconds(monsters, team);
  const slots = monsters.slice(0, 6);

  useEffect(() => {
    setTargetSeconds(defaultTarget);
  }, [defaultTarget, maxSeconds]);

  const importFile = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (file) onImportTeam(half, await file.text());
  };

  return (
    <section className="rounded-lg border bg-card px-2.5 py-2">
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5 text-xs">
        <h3 className="text-sm font-medium">{title}</h3>
        <span className="text-muted-foreground">
          {tf("{} enemies · {} kinds", [
            String(
              monsters.reduce((count, monster) => count + monster.count, 0),
            ),
            String(monsters.length),
          ])}
        </span>
        {recommendedElements.length > 0 ? (
          <span className="font-medium text-primary">
            {tf("Recommended {}", [recommendedElements.join("/")])}
          </span>
        ) : null}
        {predictionEnabled ? (
          <div className="ml-auto flex flex-wrap items-center gap-1.5">
            {team ? (
              <>
                <span className="text-muted-foreground">
                  DPS {formatStatValue(team.dps)}
                  {estimate !== null
                    ? ` · ${tf("Est. {}", [formatSeconds(estimate)])}`
                    : ""}
                </span>
                <Button
                  aria-label={t("Clear")}
                  size="icon-xs"
                  variant="ghost"
                  disabled={pending}
                  onClick={() => onClearTeam(half)}
                >
                  <X aria-hidden="true" />
                </Button>
              </>
            ) : null}
            <span className="text-muted-foreground">{t("Target")}</span>
            <input
              className="h-7 w-[68px] rounded-md border bg-background px-2 text-right tabular-nums outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/50"
              type="number"
              min={1}
              max={maxSeconds ?? 600}
              value={targetSeconds}
              onChange={(event) =>
                setTargetSeconds(
                  Math.min(
                    maxSeconds ?? 600,
                    Math.max(1, Number(event.target.value) || 1),
                  ),
                )
              }
            />
            <span className="text-muted-foreground">s</span>
            {needDps !== null ? (
              <span className="text-muted-foreground">
                {tf("Need {} DPS", [formatStatValue(needDps)])}
              </span>
            ) : null}
            {thresholds.map((threshold) => (
              <Button
                className="h-7 px-2 text-xs"
                key={threshold.stars}
                size="xs"
                variant="outline"
                onClick={() => setTargetSeconds(threshold.seconds)}
              >
                ★{threshold.stars} {formatSeconds(threshold.seconds)}
              </Button>
            ))}
            <Button
              size="xs"
              variant="outline"
              disabled={pending}
              onClick={() => onImportCurrent(half)}
            >
              {t("Import Current")}
            </Button>
            <input
              ref={fileInput}
              className="hidden"
              type="file"
              accept=".json,application/json"
              onChange={(event) => void importFile(event)}
            />
            <Button
              size="xs"
              variant="outline"
              disabled={pending}
              onClick={() => fileInput.current?.click()}
            >
              {t("Import Separately")}
            </Button>
          </div>
        ) : null}
      </div>

      {waves.length > 0 ? (
        <div className="mt-2 flex h-3 gap-0.5 overflow-hidden">
          {waves.map((wave, index) => (
            <div
              className="min-w-4 rounded-sm bg-foreground/45"
              key={wave.wave ?? "unwaved"}
              style={{
                flexGrow: wave.hp / Math.max(hp, 1),
                opacity: Math.min(0.55 + index * 0.1, 0.95),
              }}
              title={`${wave.wave === null ? t("Unwaved") : tf("Wave {}", [String(wave.wave)])} · ${wave.monsterCount} · HP ${formatStatValue(wave.hp)}`}
            />
          ))}
        </div>
      ) : null}

      {team ? <TeamRoster team={team} /> : null}

      <div className="mt-2 grid grid-cols-6 gap-1.5">
        {slots.map((monster, index) =>
          index === 5 && monsters.length > 6 ? (
            <div
              className="grid h-12 place-items-center rounded-md border text-sm text-muted-foreground"
              key="more"
              title={tf("{} more enemies not shown", [
                String(monsters.length - 5),
              ])}
            >
              +{monsters.length - 5}
            </div>
          ) : (
            <MonsterChip
              key={monster.packId}
              monster={monster}
              selected={monster.packId === selectedPackId}
              onClick={() => onSelectMonster(monster.packId)}
            />
          ),
        )}
        {Array.from({ length: Math.max(0, 6 - slots.length) }, (_, index) => (
          <div
            className="grid h-12 place-items-center rounded-md border text-xs text-muted-foreground/50"
            key={`empty:${index}`}
          >
            -
          </div>
        ))}
      </div>
    </section>
  );
}

function TeamRoster({ team }: { team: AbyssTeam }) {
  return (
    <div className="mt-2 grid grid-cols-4 gap-1.5">
      {team.members.slice(0, 4).map((member) => (
        <div
          className="flex min-w-0 items-center gap-2 rounded-md border bg-background px-2 py-1"
          key={member.id}
        >
          <span className="grid size-6 shrink-0 place-items-center rounded-md bg-muted text-xs font-medium">
            {(member.name || String(member.id)).slice(0, 1)}
          </span>
          <span className="min-w-0">
            <span className="block truncate text-xs font-medium">
              {member.name || member.id}
            </span>
            <span className="block truncate font-mono text-[10px] text-muted-foreground">
              DPS {formatStatValue(member.dps)}
            </span>
          </span>
        </div>
      ))}
    </div>
  );
}

function MonsterChip({
  monster,
  selected,
  onClick,
}: {
  monster: AbyssMonster;
  selected: boolean;
  onClick: () => void;
}) {
  const image = monsterImageUrl(monster.monsterId);
  return (
    <button
      className={cn(
        "flex h-12 min-w-0 items-center gap-1.5 rounded-md border bg-background px-1.5 text-left transition-colors hover:bg-muted",
        selected && "border-foreground ring-1 ring-foreground",
      )}
      type="button"
      title={`${monster.name} ×${monster.count}`}
      onClick={onClick}
    >
      <MonsterPortrait monster={monster} url={image} className="size-8" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-xs font-medium">
          {monster.name}
        </span>
        <span className="block truncate font-mono text-[9px] text-muted-foreground">
          {monster.wave === null ? "-" : `W${monster.wave}`} ×{monster.count} HP{" "}
          {formatStatValue(monsterTotalHp(monster))}
        </span>
      </span>
    </button>
  );
}

function AbyssMonsterDetail({ monster }: { monster: AbyssMonster }) {
  const stats = Object.entries(monster.rawProps);
  const image = monsterImageUrl(monster.monsterId);
  return (
    <section className="min-h-52 rounded-lg border bg-card p-3">
      <div className="grid grid-cols-[72px_1fr_72px] items-center">
        <MonsterPortrait monster={monster} url={image} className="size-14" />
        <div className="min-w-0 text-center">
          <h3 className="truncate text-lg font-medium">{monster.name}</h3>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {monster.half === 0
              ? t("Ascending Line")
              : monster.half === 1
                ? t("Descending Line")
                : t("Full floor config")}
            {monster.wave !== null
              ? ` · ${tf("Wave {}", [String(monster.wave)])}`
              : ""}
          </p>
        </div>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2 text-sm">
        <span className="font-medium">
          {tf("Count ×{}", [String(monster.count)])}
        </span>
        {monster.level !== null ? (
          <span className="text-muted-foreground">
            {tf("Level {}", [String(monster.level)])}
          </span>
        ) : null}
        <span>
          {tf("Total HP {}", [formatStatValue(monsterTotalHp(monster))])}
        </span>
        {monster.isBoss ? <Badge variant="outline">{t("Boss")}</Badge> : null}
      </div>
      <h4 className="mt-3 mb-1.5 text-sm font-medium">
        {t("Per-enemy stat fields")}
      </h4>
      <div className="grid grid-cols-2 overflow-hidden rounded-md max-[900px]:grid-cols-1">
        {stats.map(([key, value], index) => (
          <div
            className={cn(
              "grid grid-cols-[minmax(0,1fr)_96px] items-center gap-3 px-3 py-1 text-xs",
              Math.floor(index / 2) % 2 === 1 && "bg-muted",
              "max-[900px]:even:bg-muted max-[900px]:odd:bg-transparent",
            )}
            key={key}
          >
            <span
              className="truncate text-right text-muted-foreground"
              title={key}
            >
              {STAT_DISPLAY_NAMES[key] ?? key}
            </span>
            <span className="text-right tabular-nums">
              {formatStatValue(value)}
            </span>
          </div>
        ))}
      </div>
    </section>
  );
}

function MonsterPortrait({
  monster,
  url,
  className,
}: {
  monster: AbyssMonster;
  url: string | null;
  className?: string;
}) {
  return url ? (
    <img
      className={cn(
        "shrink-0 rounded-md border bg-background object-contain",
        className,
      )}
      src={url}
      alt=""
    />
  ) : (
    <span
      className={cn(
        "grid shrink-0 place-items-center rounded-md border bg-muted text-sm font-medium",
        className,
      )}
      aria-hidden="true"
    >
      {monster.name.trim().slice(0, 1) || "?"}
    </span>
  );
}

function AbyssValuesLoading() {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-14 shrink-0 items-center justify-between border-b px-3">
        <Skeleton className="h-4 w-64" />
        <Skeleton className="h-8 w-20" />
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="w-[228px] shrink-0 border-r p-3">
          {Array.from({ length: 8 }, (_, index) => (
            <Skeleton className="mb-2 h-8 w-full" key={index} />
          ))}
        </div>
        <div className="flex-1 p-3">
          <Skeleton className="mb-3 h-9 w-full" />
          <Skeleton className="mb-2 h-28 w-full" />
          <Skeleton className="mb-3 h-28 w-full" />
          <Skeleton className="h-72 w-full" />
        </div>
      </div>
    </div>
  );
}

function AbyssValuesError({
  error,
  onRetry,
}: {
  error: AbyssValuesCommandError;
  onRetry: () => Promise<void>;
}) {
  return (
    <div className="grid h-full place-items-center p-6">
      <Alert className="max-w-lg" variant="destructive">
        <TriangleAlert aria-hidden="true" />
        <AlertTitle>{t("Abyss monster stats table not loaded")}</AlertTitle>
        <AlertDescription>
          {tf(error.messageKey, error.messageArguments)}
          <Button
            className="mt-3"
            size="sm"
            variant="outline"
            onClick={() => void onRetry()}
          >
            <RefreshCw className="size-4" aria-hidden="true" />
            {t("Reload")}
          </Button>
        </AlertDescription>
      </Alert>
    </div>
  );
}

function FloatingError({
  error,
  onDismiss,
}: {
  error: AbyssValuesCommandError;
  onDismiss: () => void;
}) {
  return (
    <Alert
      className="fixed top-4 right-4 z-50 w-[min(28rem,calc(100vw-2rem))] bg-background/96 pr-10 shadow-lg backdrop-blur-sm"
      variant="destructive"
    >
      <TriangleAlert aria-hidden="true" />
      <AlertTitle>{t("Operation failed")}</AlertTitle>
      <AlertDescription>
        {tf(error.messageKey, error.messageArguments)}
      </AlertDescription>
      <Button
        className="absolute top-1.5 right-1.5"
        aria-label={t("Dismiss")}
        size="icon-xs"
        variant="ghost"
        onClick={onDismiss}
      >
        <X aria-hidden="true" />
      </Button>
    </Alert>
  );
}
