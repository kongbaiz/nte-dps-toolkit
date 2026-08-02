import { TechnicalContractError } from "@/lib/tauri/technical-contract";

export const MAIN_DPS_DETAIL_CONTRACT_VERSION = 1;

export type MainDpsDetailFilter =
  | "all"
  | "outgoing"
  | "incoming"
  | "characterAttributed"
  | "characterDirect"
  | "reactionDamage"
  | "sharedMechanics"
  | "unattributed";

export interface MainDpsDetailSnapshot {
  contractVersion: number;
  generation: string;
  kind: "character" | "team";
  characterId: number | null;
  characterName: string | null;
  filter: MainDpsDetailFilter;
  totalHits: number;
  totalDamage: number;
  offset: number;
  rows: MainDpsHit[];
}

export interface MainDpsHit {
  id: string;
  timestamp: number;
  characterId: number;
  characterName: string;
  direction: "outgoing" | "incoming" | "unknown";
  damage: number;
  skill: string;
  target: string;
}

export function parseMainDpsDetailSnapshot(
  value: unknown,
): MainDpsDetailSnapshot {
  const source = object(value, "main DPS detail snapshot");
  const contractVersion = integer(source.contractVersion, "contractVersion");
  if (contractVersion !== MAIN_DPS_DETAIL_CONTRACT_VERSION)
    throw new TechnicalContractError(
      `Unsupported main DPS detail contract: ${contractVersion}`,
    );
  return {
    contractVersion,
    generation: text(source.generation, "generation"),
    kind: oneOf(source.kind, ["character", "team"] as const, "kind"),
    characterId: nullableInteger(source.characterId, "characterId"),
    characterName: nullableText(source.characterName, "characterName"),
    filter: oneOf(
      source.filter,
      [
        "all",
        "outgoing",
        "incoming",
        "characterAttributed",
        "characterDirect",
        "reactionDamage",
        "sharedMechanics",
        "unattributed",
      ] as const,
      "filter",
    ),
    totalHits: integer(source.totalHits, "totalHits"),
    totalDamage: finite(source.totalDamage, "totalDamage"),
    offset: integer(source.offset, "offset"),
    rows: list(source.rows, "rows").slice(0, 250).map(parseHit),
  };
}

function parseHit(value: unknown): MainDpsHit {
  const source = object(value, "detail hit");
  return {
    id: text(source.id, "hit.id"),
    timestamp: finite(source.timestamp, "hit.timestamp"),
    characterId: integer(source.characterId, "hit.characterId"),
    characterName: text(source.characterName, "hit.characterName"),
    direction: oneOf(
      source.direction,
      ["outgoing", "incoming", "unknown"] as const,
      "hit.direction",
    ),
    damage: finite(source.damage, "hit.damage"),
    skill: text(source.skill, "hit.skill"),
    target: text(source.target, "hit.target"),
  };
}

function object(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new TechnicalContractError(`${field} must be an object`);
  return value as Record<string, unknown>;
}
function list(value: unknown, field: string): unknown[] {
  if (!Array.isArray(value))
    throw new TechnicalContractError(`${field} must be an array`);
  return value;
}
function text(value: unknown, field: string): string {
  if (typeof value !== "string")
    throw new TechnicalContractError(`${field} must be a string`);
  return value;
}
function nullableText(value: unknown, field: string): string | null {
  return value === null ? null : text(value, field);
}
function finite(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value))
    throw new TechnicalContractError(`${field} must be finite`);
  return value;
}
function integer(value: unknown, field: string): number {
  const number = finite(value, field);
  if (!Number.isInteger(number))
    throw new TechnicalContractError(`${field} must be an integer`);
  return number;
}
function nullableInteger(value: unknown, field: string): number | null {
  return value === null ? null : integer(value, field);
}
function oneOf<const T extends readonly string[]>(
  value: unknown,
  values: T,
  field: string,
): T[number] {
  const candidate = text(value, field);
  if (!values.includes(candidate))
    throw new TechnicalContractError(`${field} is unsupported`);
  return candidate as T[number];
}
