import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const RESOURCES_CONTRACT_VERSION = 1;
export const RESOURCES_MAX_ITEMS = 20_000;

export type ResourcesCommandError = TechnicalCommandError;
export type ResourceSeverity = "error" | "warning";
export type ResourceCategory =
  "character" | "skill" | "gameplayEffect" | "abyss" | "reaction" | "file";

export interface ResourceCountsSnapshot {
  characters: number;
  skillDamage: number;
  mappedEffects: number;
  semanticEffects: number;
  abyssMonsters: number;
  reactions: number;
}

export interface ResourceItemSnapshot {
  severity: ResourceSeverity;
  category: ResourceCategory;
  resourceId: string;
  displayName: string;
  messageKey: string;
  messageArguments: string[];
  suggestedSource: string;
}

export interface ResourcesSnapshot {
  contractVersion: number;
  errorCount: number;
  warningCount: number;
  itemCount: number;
  displayLimit: number;
  counts: ResourceCountsSnapshot;
  items: ResourceItemSnapshot[];
  redactedReport: string;
}

export function parseResourcesSnapshot(value: unknown): ResourcesSnapshot {
  const item = object(value, "resources snapshot");
  const contractVersion = integer(
    item.contractVersion,
    "resources.contractVersion",
  );
  if (contractVersion !== RESOURCES_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported resources contract version: ${contractVersion}`,
    );
  }
  const items = list(item.items, "resources.items");
  const displayLimit = positiveInteger(
    item.displayLimit,
    "resources.displayLimit",
  );
  if (displayLimit > RESOURCES_MAX_ITEMS) {
    throw new TechnicalContractError(
      "resources.displayLimit exceeds UI bounds",
    );
  }
  if (items.length > displayLimit) {
    throw new TechnicalContractError("resources.items exceeds UI bounds");
  }
  const itemCount = nonNegativeInteger(item.itemCount, "resources.itemCount");
  if (itemCount < items.length) {
    throw new TechnicalContractError(
      "resources.itemCount is smaller than retained items",
    );
  }
  return {
    contractVersion,
    errorCount: nonNegativeInteger(item.errorCount, "resources.errorCount"),
    warningCount: nonNegativeInteger(
      item.warningCount,
      "resources.warningCount",
    ),
    itemCount,
    displayLimit,
    counts: parseCounts(item.counts),
    items: items.map(parseItem),
    redactedReport: boundedText(
      item.redactedReport,
      "resources.redactedReport",
      2_000_000,
    ),
  };
}

export function resourcesError(error: unknown): ResourcesCommandError {
  return parseTechnicalCommandError(error);
}

function parseCounts(value: unknown): ResourceCountsSnapshot {
  const item = object(value, "resources.counts");
  return {
    characters: nonNegativeInteger(
      item.characters,
      "resources.counts.characters",
    ),
    skillDamage: nonNegativeInteger(
      item.skillDamage,
      "resources.counts.skillDamage",
    ),
    mappedEffects: nonNegativeInteger(
      item.mappedEffects,
      "resources.counts.mappedEffects",
    ),
    semanticEffects: nonNegativeInteger(
      item.semanticEffects,
      "resources.counts.semanticEffects",
    ),
    abyssMonsters: nonNegativeInteger(
      item.abyssMonsters,
      "resources.counts.abyssMonsters",
    ),
    reactions: nonNegativeInteger(item.reactions, "resources.counts.reactions"),
  };
}

function parseItem(value: unknown, index: number): ResourceItemSnapshot {
  const item = object(value, `resources.items[${index}]`);
  const messageArguments = list(
    item.messageArguments,
    `resources.items[${index}].messageArguments`,
  );
  if (messageArguments.length > 16) {
    throw new TechnicalContractError(
      `resources.items[${index}].messageArguments exceeds bounds`,
    );
  }
  return {
    severity: enumValue(
      item.severity,
      ["error", "warning"] as const,
      `resources.items[${index}].severity`,
    ),
    category: enumValue(
      item.category,
      [
        "character",
        "skill",
        "gameplayEffect",
        "abyss",
        "reaction",
        "file",
      ] as const,
      `resources.items[${index}].category`,
    ),
    resourceId: boundedText(
      item.resourceId,
      `resources.items[${index}].resourceId`,
      4_096,
    ),
    displayName: boundedText(
      item.displayName,
      `resources.items[${index}].displayName`,
      4_096,
    ),
    messageKey: boundedText(
      item.messageKey,
      `resources.items[${index}].messageKey`,
      4_096,
    ),
    messageArguments: messageArguments.map((argument, argumentIndex) =>
      boundedText(
        argument,
        `resources.items[${index}].messageArguments[${argumentIndex}]`,
        16_384,
      ),
    ),
    suggestedSource: boundedText(
      item.suggestedSource,
      `resources.items[${index}].suggestedSource`,
      16_384,
    ),
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

function boundedText(value: unknown, field: string, maxLength: number): string {
  if (typeof value !== "string" || value.length > maxLength) {
    throw new TechnicalContractError(`${field} must be bounded text`);
  }
  return value;
}

function integer(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value)) {
    throw new TechnicalContractError(`${field} must be a safe integer`);
  }
  return value;
}

function nonNegativeInteger(value: unknown, field: string): number {
  const parsed = integer(value, field);
  if (parsed < 0) {
    throw new TechnicalContractError(`${field} must be non-negative`);
  }
  return parsed;
}

function positiveInteger(value: unknown, field: string): number {
  const parsed = integer(value, field);
  if (parsed <= 0) {
    throw new TechnicalContractError(`${field} must be positive`);
  }
  return parsed;
}

function enumValue<const T extends readonly string[]>(
  value: unknown,
  allowed: T,
  field: string,
): T[number] {
  if (typeof value !== "string" || !allowed.includes(value)) {
    throw new TechnicalContractError(`${field} is invalid`);
  }
  return value as T[number];
}
