import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import {
  TechnicalContractError,
  parseTechnicalCommandError,
  type TechnicalCommandError,
} from "./technical-contract";

export const ABYSS_VALUES_CONTRACT_VERSION = 2;

const {
  array,
  boolean,
  finiteNumber,
  nonNegativeInteger: integer,
  nullable,
  nullableNonNegativeInteger: nullableInteger,
  nullableNumber,
  nullableString,
  positiveNumber,
  record,
  string,
  stringArray,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

export type AbyssHalfId = "upper" | "lower";

export interface AbyssValuesSnapshot {
  contractVersion: number;
  seasonCount: number;
  floorCount: number;
  monsterCount: number;
  seasons: AbyssSeason[];
  teams: AbyssPredictionTeams;
  currentTeamAvailable: {
    upper: boolean;
    lower: boolean;
  };
}

export interface AbyssSeason {
  season: number;
  name: string | null;
  floors: AbyssFloor[];
}

export interface AbyssFloor {
  season: number;
  seasonName: string | null;
  floor: number;
  name: string | null;
  monsterCount: number;
  waveCount: number;
  maxSeconds: number | null;
  starThresholds: AbyssStarThreshold[];
  recommendedElements: {
    firstHalf: string[];
    secondHalf: string[];
  };
  monsters: AbyssMonster[];
}

export interface AbyssStarThreshold {
  stars: number;
  seconds: number;
}

export interface AbyssMonster {
  packId: string;
  attributeId: string;
  monsterPoolId: string | null;
  monsterId: string;
  name: string;
  count: number;
  level: number | null;
  half: number | null;
  wave: number | null;
  isBoss: boolean;
  hpMaxBase: number;
  rawProps: Record<string, number>;
}

export interface AbyssPredictionTeams {
  upper: AbyssTeam | null;
  lower: AbyssTeam | null;
}

export interface AbyssTeam {
  dps: number;
  members: AbyssTeamMember[];
}

export interface AbyssTeamMember {
  id: number;
  dps: number;
  name: string;
}

export type AbyssValuesCommandError = TechnicalCommandError;

export function parseAbyssValuesSnapshot(value: unknown): AbyssValuesSnapshot {
  const snapshot = record(value, "abyss values snapshot");
  const contractVersion = integer(snapshot.contractVersion, "contractVersion");
  if (contractVersion !== ABYSS_VALUES_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `unsupported abyss values contract version ${contractVersion}`,
    );
  }
  const seasons = array(snapshot.seasons, "seasons").map(parseSeason);
  const seasonCount = integer(snapshot.seasonCount, "seasonCount");
  const floorCount = integer(snapshot.floorCount, "floorCount");
  const monsterCount = integer(snapshot.monsterCount, "monsterCount");
  const currentTeamAvailable = record(
    snapshot.currentTeamAvailable,
    "currentTeamAvailable",
  );
  if (seasonCount !== seasons.length) {
    throw new TechnicalContractError("seasonCount does not match seasons");
  }
  if (
    floorCount !==
    seasons.reduce((count, season) => count + season.floors.length, 0)
  ) {
    throw new TechnicalContractError("floorCount does not match floors");
  }
  return {
    contractVersion,
    seasonCount,
    floorCount,
    monsterCount,
    seasons,
    teams: parseAbyssPredictionTeams(snapshot.teams),
    currentTeamAvailable: {
      upper: boolean(currentTeamAvailable.upper, "currentTeamAvailable.upper"),
      lower: boolean(currentTeamAvailable.lower, "currentTeamAvailable.lower"),
    },
  };
}

export function parseAbyssPredictionTeams(
  value: unknown,
): AbyssPredictionTeams {
  const teams = record(value, "abyss prediction teams");
  return {
    upper: nullable(teams.upper, parseTeam),
    lower: nullable(teams.lower, parseTeam),
  };
}

export function abyssValuesError(error: unknown): AbyssValuesCommandError {
  return parseTechnicalCommandError(error);
}

function parseSeason(value: unknown): AbyssSeason {
  const season = record(value, "abyss season");
  return {
    season: integer(season.season, "season"),
    name: nullableString(season.name, "season.name"),
    floors: array(season.floors, "season.floors").map(parseFloor),
  };
}

function parseFloor(value: unknown): AbyssFloor {
  const floor = record(value, "abyss floor");
  const elements = record(
    floor.recommendedElements,
    "floor.recommendedElements",
  );
  return {
    season: integer(floor.season, "floor.season"),
    seasonName: nullableString(floor.seasonName, "floor.seasonName"),
    floor: integer(floor.floor, "floor.floor"),
    name: nullableString(floor.name, "floor.name"),
    monsterCount: integer(floor.monsterCount, "floor.monsterCount"),
    waveCount: integer(floor.waveCount, "floor.waveCount"),
    maxSeconds: nullableNumber(floor.maxSeconds, "floor.maxSeconds"),
    starThresholds: array(floor.starThresholds, "floor.starThresholds").map(
      (threshold) => {
        const item = record(threshold, "star threshold");
        return {
          stars: integer(item.stars, "threshold.stars"),
          seconds: finiteNumber(item.seconds, "threshold.seconds"),
        };
      },
    ),
    recommendedElements: {
      firstHalf: stringArray(elements.firstHalf, "firstHalf"),
      secondHalf: stringArray(elements.secondHalf, "secondHalf"),
    },
    monsters: array(floor.monsters, "floor.monsters").map(parseMonster),
  };
}

function parseMonster(value: unknown): AbyssMonster {
  const monster = record(value, "abyss monster");
  const rawProps = record(monster.rawProps, "monster.rawProps");
  return {
    packId: string(monster.packId, "monster.packId"),
    attributeId: string(monster.attributeId, "monster.attributeId"),
    monsterPoolId: nullableString(
      monster.monsterPoolId,
      "monster.monsterPoolId",
    ),
    monsterId: string(monster.monsterId, "monster.monsterId"),
    name: string(monster.name, "monster.name"),
    count: integer(monster.count, "monster.count"),
    level: nullableInteger(monster.level, "monster.level"),
    half: nullableInteger(monster.half, "monster.half"),
    wave: nullableInteger(monster.wave, "monster.wave"),
    isBoss: boolean(monster.isBoss, "monster.isBoss"),
    hpMaxBase: finiteNumber(monster.hpMaxBase, "monster.hpMaxBase"),
    rawProps: Object.fromEntries(
      Object.entries(rawProps).map(([key, property]) => [
        key,
        finiteNumber(property, `monster.rawProps.${key}`),
      ]),
    ),
  };
}

function parseTeam(value: unknown): AbyssTeam {
  const team = record(value, "abyss team");
  return {
    dps: positiveNumber(team.dps, "team.dps"),
    members: array(team.members, "team.members").map((value) => {
      const member = record(value, "abyss team member");
      return {
        id: integer(member.id, "member.id"),
        dps: finiteNumber(member.dps, "member.dps"),
        name: string(member.name, "member.name"),
      };
    }),
  };
}
