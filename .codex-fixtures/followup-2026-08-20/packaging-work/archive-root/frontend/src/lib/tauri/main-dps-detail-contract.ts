import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import { TechnicalContractError } from "@/lib/tauri/technical-contract";

export const MAIN_DPS_DETAIL_CONTRACT_VERSION = 5;
export const MAIN_DPS_DETAIL_MAX_QTE_SUMMARIES = 32;
export const MAIN_DPS_DETAIL_MAX_SKILLS = 250;
export const MAIN_DPS_DETAIL_MAX_ROWS = 250;
export const MAIN_DPS_DETAIL_MAX_TEXT_BYTES = 256;
export const MAIN_DPS_DETAIL_MAX_PROJECTED_TEXT_BYTES = 512 * 1024;

const {
  array: list,
  boolean: bool,
  boundedArray: boundedList,
  boundedUtf8StringAllowEmpty: boundedText,
  enumValue: oneOf,
  finiteNumber: finite,
  integer,
  nonNegativeInteger,
  nullableEnumValue: nullableOneOf,
  nullableInteger,
  nullableBoundedUtf8StringAllowEmpty: nullableBoundedText,
  record: object,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

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
  textTruncated: boolean;
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
  validateTruncation(skillTotalCount, skills.length, skillsTruncated, "skills");
  const rows = boundedList(source.rows, "rows", MAIN_DPS_DETAIL_MAX_ROWS).map(
    parseHit,
  );
  const snapshot: MainDpsDetailSnapshot = {
    contractVersion,
    generation: boundedText(source.generation, "generation", 32),
    kind: oneOf(source.kind, ["character", "team"] as const, "kind"),
    abyssHalf: nullableOneOf(
      source.abyssHalf,
      ["first", "second"] as const,
      "abyssHalf",
    ),
    characterId: nullableInteger(source.characterId, "characterId"),
    characterName: nullableBoundedText(
      source.characterName,
      "characterName",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    characterColor: nullableBoundedText(
      source.characterColor,
      "characterColor",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
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
    qteType: nullableBoundedText(
      source.qteType,
      "qteType",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    skillFilter: nullableBoundedText(
      source.skillFilter,
      "skillFilter",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
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
    textTruncated: bool(source.textTruncated, "textTruncated"),
    totalHits: integer(source.totalHits, "totalHits"),
    totalDamage: finite(source.totalDamage, "totalDamage"),
    maxRowDamage: finite(source.maxRowDamage, "maxRowDamage"),
    offset: integer(source.offset, "offset"),
    rows,
  };
  validateProjectedTextBudget(snapshot);
  return snapshot;
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
    attackType: boundedText(
      source.attackType,
      "reaction.attackType",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    hits: integer(source.hits, "reaction.hits"),
    damage: finite(source.damage, "reaction.damage"),
    sharePercent: finite(source.sharePercent, "reaction.sharePercent"),
  };
}

function parseSkillSummary(value: unknown): MainDpsSkillSummary {
  const source = object(value, "skill summary");
  return {
    id: boundedText(source.id, "skill.id", MAIN_DPS_DETAIL_MAX_TEXT_BYTES),
    name: boundedText(
      source.name,
      "skill.name",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    category: boundedText(
      source.category,
      "skill.category",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    hits: integer(source.hits, "skill.hits"),
    damage: finite(source.damage, "skill.damage"),
    sharePercent: finite(source.sharePercent, "skill.sharePercent"),
  };
}

function parseHit(value: unknown): MainDpsHit {
  const source = object(value, "detail hit");
  return {
    id: boundedText(source.id, "hit.id", MAIN_DPS_DETAIL_MAX_TEXT_BYTES),
    timestamp: finite(source.timestamp, "hit.timestamp"),
    characterId: integer(source.characterId, "hit.characterId"),
    characterName: boundedText(
      source.characterName,
      "hit.characterName",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    direction: oneOf(
      source.direction,
      ["outgoing", "incoming", "unknown"] as const,
      "hit.direction",
    ),
    damage: finite(source.damage, "hit.damage"),
    primaryDamage: finite(source.primaryDamage, "hit.primaryDamage"),
    followUpDamage: finite(source.followUpDamage, "hit.followUpDamage"),
    skillId: boundedText(
      source.skillId,
      "hit.skillId",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    skill: boundedText(
      source.skill,
      "hit.skill",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    damageType: boundedText(
      source.damageType,
      "hit.damageType",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    typeLabel: boundedText(
      source.typeLabel,
      "hit.typeLabel",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    reactionTextKey: nullableInteger(
      source.reactionTextKey,
      "hit.reactionTextKey",
    ),
    damageDigitKey: nullableBoundedText(
      source.damageDigitKey,
      "hit.damageDigitKey",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    followUpDamageDigitKey: nullableBoundedText(
      source.followUpDamageDigitKey,
      "hit.followUpDamageDigitKey",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    target: boundedText(
      source.target,
      "hit.target",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    targetMonsterId: nullableBoundedText(
      source.targetMonsterId,
      "hit.targetMonsterId",
      MAIN_DPS_DETAIL_MAX_TEXT_BYTES,
    ),
    targetHpAfter: finite(source.targetHpAfter, "hit.targetHpAfter"),
    targetMaxHp: finite(source.targetMaxHp, "hit.targetMaxHp"),
    targetHpPercent: finite(source.targetHpPercent, "hit.targetHpPercent"),
  };
}

function validateProjectedTextBudget(snapshot: MainDpsDetailSnapshot): void {
  const encoder = new TextEncoder();
  let bytes = 0;
  const add = (value: string | null): void => {
    if (value !== null) bytes += encoder.encode(value).byteLength;
  };
  add(snapshot.characterName);
  add(snapshot.characterColor);
  add(snapshot.qteType);
  add(snapshot.skillFilter);
  for (const summary of snapshot.qteSummaries) add(summary.attackType);
  for (const skill of snapshot.skills) {
    add(skill.id);
    add(skill.name);
    add(skill.category);
  }
  for (const hit of snapshot.rows) {
    add(hit.id);
    add(hit.characterName);
    add(hit.skillId);
    add(hit.skill);
    add(hit.damageType);
    add(hit.typeLabel);
    add(hit.damageDigitKey);
    add(hit.followUpDamageDigitKey);
    add(hit.target);
    add(hit.targetMonsterId);
  }
  if (bytes > MAIN_DPS_DETAIL_MAX_PROJECTED_TEXT_BYTES)
    throw new TechnicalContractError(
      `main DPS detail text exceeds ${MAIN_DPS_DETAIL_MAX_PROJECTED_TEXT_BYTES} UTF-8 bytes`,
    );
}

function validateTruncation(
  totalCount: number,
  returnedCount: number,
  truncated: boolean,
  field: string,
): void {
  if (totalCount < returnedCount)
    throw new TechnicalContractError(
      `${field} total count is below returned rows`,
    );
  if (truncated !== totalCount > returnedCount)
    throw new TechnicalContractError(
      `${field} truncation metadata is inconsistent`,
    );
}
