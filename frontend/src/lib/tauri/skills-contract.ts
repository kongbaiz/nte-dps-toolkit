import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const SKILLS_CONTRACT_VERSION = 1;
export const SKILLS_MAX_CHARACTERS = 64;
export const SKILLS_MAX_ROWS = 4096;
export const SKILLS_MAX_UNMAPPED_EFFECTS = 512;

export type SkillsScope = "all" | "upper" | "lower";
export type SkillsCommandError = TechnicalCommandError;

export interface SkillsCharacter {
  id: number;
  name: string;
  color: string;
  damage: number;
  entries: number;
}

export interface SkillsRow {
  id: string;
  characterId: number;
  characterName: string;
  name: string;
  category: string;
  abilityName: string | null;
  damageName: string | null;
  gameplayEffectIndex: number | null;
  gameplayEffectName: string | null;
  followUp: boolean;
  hits: string;
  damage: number;
}

export interface SkillsDiagnostics {
  unknownCharacterCount: string;
  unknownCharacterHits: string;
  unknownDirectionHits: string;
  unknownDirectionDamage: number;
  unmappedSkillRows: string;
  unmappedSkillHits: string;
  unmappedSkillDamage: number;
  unmappedGameplayEffects: Array<{
    index: number;
    hits: string;
    damage: number;
  }>;
}

export interface SkillsSnapshot {
  contractVersion: number;
  generation: string;
  scope: SkillsScope;
  hasData: boolean;
  totalDamage: number;
  totalHits: string;
  characters: SkillsCharacter[];
  rows: SkillsRow[];
  diagnostics: SkillsDiagnostics;
}

export function parseSkillsSnapshot(value: unknown): SkillsSnapshot {
  const item = object(value, "skills snapshot");
  const contractVersion = integer(
    item.contractVersion,
    "skills.contractVersion",
  );
  if (contractVersion !== SKILLS_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported skills contract version: ${contractVersion}`,
    );
  }
  const characters = list(item.characters, "skills.characters");
  const rows = list(item.rows, "skills.rows");
  if (characters.length > SKILLS_MAX_CHARACTERS) {
    throw new TechnicalContractError(
      "skills.characters exceeds display bounds",
    );
  }
  if (rows.length > SKILLS_MAX_ROWS) {
    throw new TechnicalContractError("skills.rows exceeds display bounds");
  }
  return {
    contractVersion,
    generation: decimal(item.generation, "skills.generation"),
    scope: enumValue(
      item.scope,
      ["all", "upper", "lower"] as const,
      "skills.scope",
    ),
    hasData: flag(item.hasData, "skills.hasData"),
    totalDamage: nonNegative(item.totalDamage, "skills.totalDamage"),
    totalHits: decimal(item.totalHits, "skills.totalHits"),
    characters: characters.map((value, index) => {
      const row = object(value, `skills.characters[${index}]`);
      return {
        id: nonNegativeInteger(row.id, `skills.characters[${index}].id`),
        name: text(row.name, `skills.characters[${index}].name`),
        color: cssHex(row.color, `skills.characters[${index}].color`),
        damage: nonNegative(row.damage, `skills.characters[${index}].damage`),
        entries: nonNegativeInteger(
          row.entries,
          `skills.characters[${index}].entries`,
        ),
      };
    }),
    rows: rows.map((value, index) => {
      const row = object(value, `skills.rows[${index}]`);
      return {
        id: boundedText(row.id, `skills.rows[${index}].id`, 96),
        characterId: nonNegativeInteger(
          row.characterId,
          `skills.rows[${index}].characterId`,
        ),
        characterName: text(
          row.characterName,
          `skills.rows[${index}].characterName`,
        ),
        name: text(row.name, `skills.rows[${index}].name`),
        category: text(row.category, `skills.rows[${index}].category`),
        abilityName: optionalText(
          row.abilityName,
          `skills.rows[${index}].abilityName`,
        ),
        damageName: optionalText(
          row.damageName,
          `skills.rows[${index}].damageName`,
        ),
        gameplayEffectIndex: optionalNonNegativeInteger(
          row.gameplayEffectIndex,
          `skills.rows[${index}].gameplayEffectIndex`,
        ),
        gameplayEffectName: optionalText(
          row.gameplayEffectName,
          `skills.rows[${index}].gameplayEffectName`,
        ),
        followUp: flag(row.followUp, `skills.rows[${index}].followUp`),
        hits: decimal(row.hits, `skills.rows[${index}].hits`),
        damage: nonNegative(row.damage, `skills.rows[${index}].damage`),
      };
    }),
    diagnostics: parseDiagnostics(item.diagnostics),
  };
}

export function parseSkillsEvent(value: unknown): SkillsSnapshot {
  const item = object(value, "skills event");
  if (item.event !== "snapshot") {
    throw new TechnicalContractError("Unsupported skills event");
  }
  return parseSkillsSnapshot(item.payload);
}

export function skillsError(error: unknown): SkillsCommandError {
  return parseTechnicalCommandError(error);
}

function parseDiagnostics(value: unknown): SkillsDiagnostics {
  const item = object(value, "skills.diagnostics");
  const effects = list(
    item.unmappedGameplayEffects,
    "skills.diagnostics.unmappedGameplayEffects",
  );
  if (effects.length > SKILLS_MAX_UNMAPPED_EFFECTS) {
    throw new TechnicalContractError(
      "skills diagnostics exceeds display bounds",
    );
  }
  return {
    unknownCharacterCount: decimal(
      item.unknownCharacterCount,
      "skills.diagnostics.unknownCharacterCount",
    ),
    unknownCharacterHits: decimal(
      item.unknownCharacterHits,
      "skills.diagnostics.unknownCharacterHits",
    ),
    unknownDirectionHits: decimal(
      item.unknownDirectionHits,
      "skills.diagnostics.unknownDirectionHits",
    ),
    unknownDirectionDamage: nonNegative(
      item.unknownDirectionDamage,
      "skills.diagnostics.unknownDirectionDamage",
    ),
    unmappedSkillRows: decimal(
      item.unmappedSkillRows,
      "skills.diagnostics.unmappedSkillRows",
    ),
    unmappedSkillHits: decimal(
      item.unmappedSkillHits,
      "skills.diagnostics.unmappedSkillHits",
    ),
    unmappedSkillDamage: nonNegative(
      item.unmappedSkillDamage,
      "skills.diagnostics.unmappedSkillDamage",
    ),
    unmappedGameplayEffects: effects.map((value, index) => {
      const row = object(
        value,
        `skills.diagnostics.unmappedGameplayEffects[${index}]`,
      );
      return {
        index: nonNegativeInteger(
          row.index,
          `skills.diagnostics.unmappedGameplayEffects[${index}].index`,
        ),
        hits: decimal(
          row.hits,
          `skills.diagnostics.unmappedGameplayEffects[${index}].hits`,
        ),
        damage: nonNegative(
          row.damage,
          `skills.diagnostics.unmappedGameplayEffects[${index}].damage`,
        ),
      };
    }),
  };
}

function object(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TechnicalContractError(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}

function list(value: unknown, field: string): unknown[] {
  if (!Array.isArray(value)) {
    throw new TechnicalContractError(`${field} must be an array`);
  }
  return value;
}

function text(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new TechnicalContractError(`${field} must be a string`);
  }
  return value;
}

function boundedText(value: unknown, field: string, maxLength: number): string {
  const parsed = text(value, field);
  if (parsed.length === 0 || parsed.length > maxLength) {
    throw new TechnicalContractError(`${field} has an invalid length`);
  }
  return parsed;
}

function optionalText(value: unknown, field: string): string | null {
  return value === null || value === undefined ? null : text(value, field);
}

function flag(value: unknown, field: string): boolean {
  if (typeof value !== "boolean") {
    throw new TechnicalContractError(`${field} must be a boolean`);
  }
  return value;
}

function number(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new TechnicalContractError(`${field} must be finite`);
  }
  return value;
}

function nonNegative(value: unknown, field: string): number {
  const parsed = number(value, field);
  if (parsed < 0) {
    throw new TechnicalContractError(`${field} must not be negative`);
  }
  return parsed;
}

function integer(value: unknown, field: string): number {
  const parsed = number(value, field);
  if (!Number.isInteger(parsed)) {
    throw new TechnicalContractError(`${field} must be an integer`);
  }
  return parsed;
}

function nonNegativeInteger(value: unknown, field: string): number {
  const parsed = integer(value, field);
  if (parsed < 0) {
    throw new TechnicalContractError(`${field} must not be negative`);
  }
  return parsed;
}

function optionalNonNegativeInteger(
  value: unknown,
  field: string,
): number | null {
  return value === null || value === undefined
    ? null
    : nonNegativeInteger(value, field);
}

function decimal(value: unknown, field: string): string {
  const parsed = text(value, field);
  if (!/^(0|[1-9]\d*)$/.test(parsed)) {
    throw new TechnicalContractError(`${field} must be a decimal string`);
  }
  return parsed;
}

function enumValue<const T extends readonly string[]>(
  value: unknown,
  options: T,
  field: string,
): T[number] {
  if (typeof value !== "string" || !options.includes(value)) {
    throw new TechnicalContractError(`${field} has an unsupported value`);
  }
  return value as T[number];
}

function cssHex(value: unknown, field: string): string {
  const parsed = text(value, field);
  if (!/^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/.test(parsed)) {
    throw new TechnicalContractError(`${field} must be a CSS hex color`);
  }
  return parsed;
}
