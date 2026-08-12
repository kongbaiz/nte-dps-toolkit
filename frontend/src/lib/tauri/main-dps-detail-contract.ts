import { TechnicalContractError } from "@/lib/tauri/technical-contract";

export const MAIN_DPS_DETAIL_CONTRACT_VERSION = 4;
export const MAIN_DPS_DETAIL_MAX_QTE_SUMMARIES = 32;
export const MAIN_DPS_DETAIL_MAX_SKILLS = 250;
export const MAIN_DPS_DETAIL_MAX_ROWS = 250;

export type MainDpsDetailFilter =
  | "all"
  | "outgoing"
  | "incoming"
  | "characterAttributed"
  | "characterDirect"
  | "reactionDamage"
  | "sharedMechanics"
  | "unattributed"
  | "qteType";

export interface MainDpsDetailSnapshot {
  contractVersion: number;
  generation: string;
  kind: "character" | "team";
  abyssHalf: "first" | "second" | null;
  characterId: number | null;
  characterName: string | null;
  characterColor: string | null;
  filter: MainDpsDetailFilter;
  qteType: string | null;
  skillFilter: string | null;
  columns: MainDpsDetailColumns;
  actions: MainDpsDetailActions;
  metrics: MainDpsDetailMetrics;
  direction: MainDpsDirectionSummary;
  hitTypes: MainDpsFilterSummary[];
  attribution: MainDpsAttributionSummary;
  qteSummaries: MainDpsQteSummary[];
  qteSummaryTotalCount: number;
  qteSummariesTruncated: boolean;
  skills: MainDpsSkillSummary[];
  skillTotalCount: number;
  skillsTruncated: boolean;
  totalHits: number;
  totalDamage: number;
  maxRowDamage: number;
  offset: number;
  rows: MainDpsHit[];
}

export interface MainDpsDetailColumns {
  showTime: boolean;
  showCharacter: boolean;
  showType: boolean;
  showDamage: boolean;
  showTarget: boolean;
  timeWidth: number;
  characterWidth: number;
  typeWidth: number;
  damageWidth: number;
  targetWidth: number;
}

export interface MainDpsDetailActions {
  canStartCapture: boolean;
  canImportReplay: boolean;
}

export interface MainDpsDetailMetrics {
  totalOutput: number;
  dps: number;
  outputCount: number;
  incomingCount: number;
  totalDamageTaken: number;
  durationSeconds: number;
}

export interface MainDpsDirectionSummary {
  confirmedOutput: number;
  confirmedHits: number;
  candidateOutput: number;
  candidateHits: number;
  incomingOutput: number;
  incomingHits: number;
  candidateSharePercent: number;
}

export interface MainDpsFilterSummary {
  id: "all" | "outgoing" | "incoming";
  hits: number;
  damage: number;
}

export interface MainDpsAttributionSummary {
  totalDamage: number;
  characterDamage: number;
  characterFilter: "characterAttributed" | "characterDirect";
  reactionDamage: number;
  sharedDamage: number;
  unattributedDamage: number;
  separateReactionDamage: boolean;
}

export interface MainDpsQteSummary {
  attackType: string;
  hits: number;
  damage: number;
  sharePercent: number;
}

export interface MainDpsSkillSummary {
  id: string;
  name: string;
  category: string;
  hits: number;
  damage: number;
  sharePercent: number;
}

export interface MainDpsHit {
  id: string;
  timestamp: number;
  characterId: number;
  characterName: string;
  direction: "outgoing" | "incoming" | "unknown";
  damage: number;
  primaryDamage: number;
  followUpDamage: number;
  skillId: string;
  skill: string;
  damageType: string;
  typeLabel: string;
  reactionTextKey: number | null;
  damageDigitKey: string | null;
  followUpDamageDigitKey: string | null;
  target: string;
  targetMonsterId: string | null;
  targetHpAfter: number;
  targetMaxHp: number;
  targetHpPercent: number;
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
  const qteSummaries = boundedList(
    source.qteSummaries,
    "qteSummaries",
    MAIN_DPS_DETAIL_MAX_QTE_SUMMARIES,
  ).map(parseQteSummary);
  const qteSummaryTotalCount = nonNegativeInteger(
    source.qteSummaryTotalCount,
    "qteSummaryTotalCount",
  );
  const qteSummariesTruncated = bool(
    source.qteSummariesTruncated,
    "qteSummariesTruncated",
  );
  validateTruncation(
    qteSummaryTotalCount,
    qteSummaries.length,
    qteSummariesTruncated,
    "qteSummaries",
  );
  const skills = boundedList(
    source.skills,
    "skills",
    MAIN_DPS_DETAIL_MAX_SKILLS,
  ).map(parseSkillSummary);
  const skillTotalCount = nonNegativeInteger(
    source.skillTotalCount,
    "skillTotalCount",
  );
  const skillsTruncated = bool(source.skillsTruncated, "skillsTruncated");
  validateTruncation(
    skillTotalCount,
    skills.length,
    skillsTruncated,
    "skills",
  );
  const rows = boundedList(
    source.rows,
    "rows",
    MAIN_DPS_DETAIL_MAX_ROWS,
  ).map(parseHit);
  return {
    contractVersion,
    generation: text(source.generation, "generation"),
    kind: oneOf(source.kind, ["character", "team"] as const, "kind"),
    abyssHalf: nullableOneOf(
      source.abyssHalf,
      ["first", "second"] as const,
      "abyssHalf",
    ),
    characterId: nullableInteger(source.characterId, "characterId"),
    characterName: nullableText(source.characterName, "characterName"),
    characterColor: nullableText(source.characterColor, "characterColor"),
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
        "qteType",
      ] as const,
      "filter",
    ),
    qteType: nullableText(source.qteType, "qteType"),
    skillFilter: nullableText(source.skillFilter, "skillFilter"),
    columns: parseColumns(source.columns),
    actions: parseActions(source.actions),
    metrics: parseMetrics(source.metrics),
    direction: parseDirection(source.direction),
    hitTypes: list(source.hitTypes, "hitTypes").map(parseFilterSummary),
    attribution: parseAttribution(source.attribution),
    qteSummaries,
    qteSummaryTotalCount,
    qteSummariesTruncated,
    skills,
    skillTotalCount,
    skillsTruncated,
    totalHits: integer(source.totalHits, "totalHits"),
    totalDamage: finite(source.totalDamage, "totalDamage"),
    maxRowDamage: finite(source.maxRowDamage, "maxRowDamage"),
    offset: integer(source.offset, "offset"),
    rows,
  };
}

function parseColumns(value: unknown): MainDpsDetailColumns {
  const source = object(value, "columns");
  return {
    showTime: bool(source.showTime, "columns.showTime"),
    showCharacter: bool(source.showCharacter, "columns.showCharacter"),
    showType: bool(source.showType, "columns.showType"),
    showDamage: bool(source.showDamage, "columns.showDamage"),
    showTarget: bool(source.showTarget, "columns.showTarget"),
    timeWidth: integer(source.timeWidth, "columns.timeWidth"),
    characterWidth: integer(source.characterWidth, "columns.characterWidth"),
    typeWidth: integer(source.typeWidth, "columns.typeWidth"),
    damageWidth: integer(source.damageWidth, "columns.damageWidth"),
    targetWidth: integer(source.targetWidth, "columns.targetWidth"),
  };
}

function parseActions(value: unknown): MainDpsDetailActions {
  const source = object(value, "actions");
  return {
    canStartCapture: bool(source.canStartCapture, "actions.canStartCapture"),
    canImportReplay: bool(source.canImportReplay, "actions.canImportReplay"),
  };
}

function parseMetrics(value: unknown): MainDpsDetailMetrics {
  const source = object(value, "metrics");
  return {
    totalOutput: finite(source.totalOutput, "metrics.totalOutput"),
    dps: finite(source.dps, "metrics.dps"),
    outputCount: integer(source.outputCount, "metrics.outputCount"),
    incomingCount: integer(source.incomingCount, "metrics.incomingCount"),
    totalDamageTaken: finite(
      source.totalDamageTaken,
      "metrics.totalDamageTaken",
    ),
    durationSeconds: finite(source.durationSeconds, "metrics.durationSeconds"),
  };
}

function parseDirection(value: unknown): MainDpsDirectionSummary {
  const source = object(value, "direction");
  return {
    confirmedOutput: finite(
      source.confirmedOutput,
      "direction.confirmedOutput",
    ),
    confirmedHits: integer(source.confirmedHits, "direction.confirmedHits"),
    candidateOutput: finite(
      source.candidateOutput,
      "direction.candidateOutput",
    ),
    candidateHits: integer(source.candidateHits, "direction.candidateHits"),
    incomingOutput: finite(source.incomingOutput, "direction.incomingOutput"),
    incomingHits: integer(source.incomingHits, "direction.incomingHits"),
    candidateSharePercent: finite(
      source.candidateSharePercent,
      "direction.candidateSharePercent",
    ),
  };
}

function parseFilterSummary(value: unknown): MainDpsFilterSummary {
  const source = object(value, "filter summary");
  return {
    id: oneOf(source.id, ["all", "outgoing", "incoming"] as const, "filter.id"),
    hits: integer(source.hits, "filter.hits"),
    damage: finite(source.damage, "filter.damage"),
  };
}

function parseAttribution(value: unknown): MainDpsAttributionSummary {
  const source = object(value, "attribution");
  return {
    totalDamage: finite(source.totalDamage, "attribution.totalDamage"),
    characterDamage: finite(
      source.characterDamage,
      "attribution.characterDamage",
    ),
    characterFilter: oneOf(
      source.characterFilter,
      ["characterAttributed", "characterDirect"] as const,
      "attribution.characterFilter",
    ),
    reactionDamage: finite(source.reactionDamage, "attribution.reactionDamage"),
    sharedDamage: finite(source.sharedDamage, "attribution.sharedDamage"),
    unattributedDamage: finite(
      source.unattributedDamage,
      "attribution.unattributedDamage",
    ),
    separateReactionDamage: bool(
      source.separateReactionDamage,
      "attribution.separateReactionDamage",
    ),
  };
}

function parseQteSummary(value: unknown): MainDpsQteSummary {
  const source = object(value, "reaction summary");
  return {
    attackType: text(source.attackType, "reaction.attackType"),
    hits: integer(source.hits, "reaction.hits"),
    damage: finite(source.damage, "reaction.damage"),
    sharePercent: finite(source.sharePercent, "reaction.sharePercent"),
  };
}

function parseSkillSummary(value: unknown): MainDpsSkillSummary {
  const source = object(value, "skill summary");
  return {
    id: text(source.id, "skill.id"),
    name: text(source.name, "skill.name"),
    category: text(source.category, "skill.category"),
    hits: integer(source.hits, "skill.hits"),
    damage: finite(source.damage, "skill.damage"),
    sharePercent: finite(source.sharePercent, "skill.sharePercent"),
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
    primaryDamage: finite(source.primaryDamage, "hit.primaryDamage"),
    followUpDamage: finite(source.followUpDamage, "hit.followUpDamage"),
    skillId: text(source.skillId, "hit.skillId"),
    skill: text(source.skill, "hit.skill"),
    damageType: text(source.damageType, "hit.damageType"),
    typeLabel: text(source.typeLabel, "hit.typeLabel"),
    reactionTextKey: nullableInteger(
      source.reactionTextKey,
      "hit.reactionTextKey",
    ),
    damageDigitKey: nullableText(source.damageDigitKey, "hit.damageDigitKey"),
    followUpDamageDigitKey: nullableText(
      source.followUpDamageDigitKey,
      "hit.followUpDamageDigitKey",
    ),
    target: text(source.target, "hit.target"),
    targetMonsterId: nullableText(
      source.targetMonsterId,
      "hit.targetMonsterId",
    ),
    targetHpAfter: finite(source.targetHpAfter, "hit.targetHpAfter"),
    targetMaxHp: finite(source.targetMaxHp, "hit.targetMaxHp"),
    targetHpPercent: finite(source.targetHpPercent, "hit.targetHpPercent"),
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
function boundedList(value: unknown, field: string, limit: number): unknown[] {
  const rows = list(value, field);
  if (rows.length > limit)
    throw new TechnicalContractError(`${field} exceeds the contract limit`);
  return rows;
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
function nonNegativeInteger(value: unknown, field: string): number {
  const number = integer(value, field);
  if (number < 0)
    throw new TechnicalContractError(`${field} must be non-negative`);
  return number;
}
function validateTruncation(
  totalCount: number,
  returnedCount: number,
  truncated: boolean,
  field: string,
): void {
  if (totalCount < returnedCount)
    throw new TechnicalContractError(`${field} total count is below returned rows`);
  if (truncated !== (totalCount > returnedCount))
    throw new TechnicalContractError(`${field} truncation metadata is inconsistent`);
}
function nullableInteger(value: unknown, field: string): number | null {
  return value === null ? null : integer(value, field);
}
function bool(value: unknown, field: string): boolean {
  if (typeof value !== "boolean")
    throw new TechnicalContractError(`${field} must be a boolean`);
  return value;
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
function nullableOneOf<const T extends readonly string[]>(
  value: unknown,
  values: T,
  field: string,
): T[number] | null {
  return value === null ? null : oneOf(value, values, field);
}
