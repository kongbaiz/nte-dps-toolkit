import type {
  AbyssFloor,
  AbyssMonster,
  AbyssSeason,
  AbyssTeam,
} from "@/lib/tauri/abyss-values-contract";

export interface AbyssFloorKey {
  season: number;
  floor: number;
}

export interface AbyssWaveSummary {
  wave: number | null;
  hp: number;
  monsterCount: number;
}

export function initialFloorKey(
  seasons: readonly AbyssSeason[],
): AbyssFloorKey | null {
  const latest = seasons.at(-1);
  const floor = latest?.floors[0];
  return floor
    ? {
        season: floor.season,
        floor: floor.floor,
      }
    : null;
}

export function findFloor(
  seasons: readonly AbyssSeason[],
  key: AbyssFloorKey | null,
): AbyssFloor | null {
  if (!key) return null;
  return (
    seasons
      .find((season) => season.season === key.season)
      ?.floors.find((floor) => floor.floor === key.floor) ?? null
  );
}

export function filterMonsters(
  monsters: readonly AbyssMonster[],
  query: string,
): AbyssMonster[] {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return [...monsters];
  return monsters.filter((monster) =>
    [
      monster.name,
      monster.packId,
      monster.attributeId,
      monster.monsterId,
      monster.monsterPoolId ?? "",
    ].some((value) => value.toLocaleLowerCase().includes(normalized)),
  );
}

export function monstersForHalf(
  monsters: readonly AbyssMonster[],
  half: 0 | 1,
): AbyssMonster[] {
  return monsters.filter((monster) => monster.half === half);
}

export function monsterTotalHp(monster: AbyssMonster): number {
  return monster.hpMaxBase * monster.count;
}

export function lineTotalHp(monsters: readonly AbyssMonster[]): number {
  return monsters.reduce(
    (total, monster) => total + monsterTotalHp(monster),
    0,
  );
}

export function requiredDps(
  monsters: readonly AbyssMonster[],
  seconds: number,
): number | null {
  const hp = lineTotalHp(monsters);
  return Number.isFinite(seconds) && seconds > 0 && hp > 0
    ? hp / seconds
    : null;
}

export function predictedSeconds(
  monsters: readonly AbyssMonster[],
  team: AbyssTeam | null,
): number | null {
  const hp = lineTotalHp(monsters);
  return team && team.dps > 0 && hp > 0 ? hp / team.dps : null;
}

export function teamHeaderMembers(team: AbyssTeam): AbyssTeam["members"] {
  return team.members.slice(0, 4);
}

export function summarizeWaves(
  monsters: readonly AbyssMonster[],
): AbyssWaveSummary[] {
  const waves = new Map<number | null, AbyssWaveSummary>();
  for (const monster of monsters) {
    const summary = waves.get(monster.wave) ?? {
      wave: monster.wave,
      hp: 0,
      monsterCount: 0,
    };
    summary.hp += monsterTotalHp(monster);
    summary.monsterCount += monster.count;
    waves.set(monster.wave, summary);
  }
  return [...waves.values()].sort(
    (left, right) =>
      (left.wave ?? Number.MAX_SAFE_INTEGER) -
      (right.wave ?? Number.MAX_SAFE_INTEGER),
  );
}

export function formatStatValue(value: number): string {
  if (Math.abs(value) >= 1000 || Number.isInteger(value)) {
    return new Intl.NumberFormat(undefined, {
      maximumFractionDigits: 0,
    }).format(value);
  }
  return value.toFixed(2);
}

export function formatSeconds(seconds: number): string {
  if (seconds >= 60) {
    const minutes = Math.floor(seconds / 60);
    const remainder = seconds - minutes * 60;
    return `${minutes}m${remainder.toFixed(1).padStart(4, "0")}s`;
  }
  return `${seconds.toFixed(1)}s`;
}

const monsterImages = import.meta.glob<string>("@res/images/monsters/*.png", {
  eager: true,
  query: "?url",
  import: "default",
});

const monsterImagesByStem = new Map(
  Object.entries(monsterImages).map(([path, url]) => [
    path
      .slice(path.lastIndexOf("/") + 1)
      .replace(/\.png$/i, "")
      .toLocaleLowerCase(),
    url,
  ]),
);

export function monsterImageUrl(monsterId: string): string | null {
  for (const stem of monsterImageStemCandidates(monsterId)) {
    const url = monsterImagesByStem.get(stem.toLocaleLowerCase());
    if (url) return url;
  }
  return null;
}

export function monsterImageStemCandidates(monsterId: string): string[] {
  const base = monsterId.replace(/\.[^.]+$/, "");
  const rawCandidates = [base];
  let current = base;
  for (;;) {
    const next = current.replace(/(?:_abyss|_bp|_bf|_b)$/i, "");
    if (next === current) break;
    rawCandidates.push(next);
    current = next;
  }
  const withoutColor = current.replace(/_(?:blue|red)$/i, "");
  if (withoutColor !== current) rawCandidates.push(withoutColor);
  const summon = current.split(/_summon/i)[0];
  if (summon && summon !== current) rawCandidates.push(summon);
  const double = current.split(/_double_/i)[0];
  if (double && double !== current) rawCandidates.push(double);
  const withoutNumeric = current.replace(/_\d+$/, "");
  if (withoutNumeric !== current && withoutNumeric.includes("_")) {
    rawCandidates.push(withoutNumeric);
  }

  // Resource filenames normalize numeric path segments (`019` -> `19`,
  // `030` -> `30`) while the authoritative monster ids preserve their UE
  // zero padding. Keep raw candidates first for exact assets, then mirror the
  // canonicalization used by the egui portrait loader.
  const candidates = rawCandidates.flatMap((candidate) => [
    candidate,
    canonicalMonsterImageStem(candidate),
  ]);
  return [...new Set(candidates)];
}

function canonicalMonsterImageStem(stem: string): string {
  return stem
    .split("_")
    .filter(Boolean)
    .map((part) => (/^\d+$/.test(part) ? String(Number(part)) : part))
    .join("_");
}
