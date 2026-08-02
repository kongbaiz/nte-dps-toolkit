import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const EMPTY_CURTAIN_CONTRACT_VERSION = 2;
export const EMPTY_CURTAIN_MAX_CHARACTERS = 64;
export const EMPTY_CURTAIN_MAX_ITEMS = 4096;

export type EmptyCurtainCommandError = TechnicalCommandError;
export type EquipmentKind = "module" | "core" | null;
export type EquipmentQuality = "blue" | "purple" | "orange" | null;
export type EquipmentAction =
  "lock" | "unlock" | "discard" | "restore" | "unequip" | "equip";
export type CharacterEquipmentAction = "unequip-all" | "one-click";

export interface ItemUid {
  slot: number;
  serial: number;
}

export interface EmptyCurtainCharacter {
  uid: ItemUid;
  characterId: number;
  name: string;
}

export interface EmptyCurtainStat {
  property: string;
  label: string;
  value: number;
  percent: boolean;
  main: boolean;
  unlockLevel: number | null;
  unlocked: boolean;
}

export interface EmptyCurtainSetEffect {
  count: number;
  text: string;
}

export interface EmptyCurtainPlacement {
  row: number;
  column: number;
}

export interface EmptyCurtainItem {
  uid: ItemUid;
  itemId: string;
  filterId: string;
  kind: EquipmentKind;
  quality: EquipmentQuality;
  name: string;
  icon: string | null;
  level: number;
  maxLevel: number | null;
  locked: boolean;
  discarded: boolean;
  equippedCharacterUid: ItemUid | null;
  equippedCharacterId: number | null;
  equippedPlacement: EmptyCurtainPlacement | null;
  stats: EmptyCurtainStat[];
  setName: string | null;
  setEffects: EmptyCurtainSetEffect[];
}

export interface EmptyCurtainOperation {
  status: "idle" | "pending" | "success" | "error";
  messageKey: string;
  messageArguments: string[];
}

export interface EmptyCurtainSnapshot {
  contractVersion: number;
  generation: string;
  observedAtUnixMs: string;
  hasData: boolean;
  complete: boolean;
  characters: EmptyCurtainCharacter[];
  items: EmptyCurtainItem[];
  operation: EmptyCurtainOperation;
}

export interface EmptyCurtainFileResult {
  completed: boolean;
  snapshot: EmptyCurtainSnapshot;
}

export function parseEmptyCurtainSnapshot(
  value: unknown,
): EmptyCurtainSnapshot {
  const data = object(value, "emptyCurtain");
  const contractVersion = uint(data.contractVersion, "contractVersion");
  if (contractVersion !== EMPTY_CURTAIN_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported Console equipment contract version: ${contractVersion}`,
    );
  }
  const characters = list(data.characters, "characters");
  const items = list(data.items, "items");
  if (characters.length > EMPTY_CURTAIN_MAX_CHARACTERS) {
    throw new TechnicalContractError("characters exceeds display bounds");
  }
  if (items.length > EMPTY_CURTAIN_MAX_ITEMS) {
    throw new TechnicalContractError("items exceeds display bounds");
  }
  return {
    contractVersion,
    generation: decimal(data.generation, "generation"),
    observedAtUnixMs: decimal(data.observedAtUnixMs, "observedAtUnixMs"),
    hasData: flag(data.hasData, "hasData"),
    complete: flag(data.complete, "complete"),
    characters: characters.map((value, index) => {
      const row = object(value, `characters[${index}]`);
      return {
        uid: uid(row.uid, `characters[${index}].uid`),
        characterId: uint(row.characterId, `characters[${index}].characterId`),
        name: text(row.name, `characters[${index}].name`),
      };
    }),
    items: items.map(parseItem),
    operation: parseOperation(data.operation),
  };
}

export function parseEmptyCurtainEvent(value: unknown): EmptyCurtainSnapshot {
  const event = object(value, "emptyCurtainEvent");
  if (event.event !== "snapshot") {
    throw new TechnicalContractError("Unsupported Console equipment event");
  }
  return parseEmptyCurtainSnapshot(event.payload);
}

export function parseEmptyCurtainPositions(
  value: unknown,
): EmptyCurtainPlacement[] {
  const positions = list(value, "positions");
  if (positions.length > 64) {
    throw new TechnicalContractError("positions exceeds display bounds");
  }
  return positions.map((value, index) =>
    placement(value, `positions[${index}]`),
  );
}

export function parseEmptyCurtainFileResult(
  value: unknown,
): EmptyCurtainFileResult {
  const result = object(value, "fileResult");
  return {
    completed: flag(result.completed, "fileResult.completed"),
    snapshot: parseEmptyCurtainSnapshot(result.snapshot),
  };
}

export function emptyCurtainError(error: unknown): EmptyCurtainCommandError {
  return parseTechnicalCommandError(error);
}

export function itemUidKey(value: ItemUid): string {
  return `${value.slot}:${value.serial}`;
}

function parseItem(value: unknown, index: number): EmptyCurtainItem {
  const field = `items[${index}]`;
  const row = object(value, field);
  const stats = list(row.stats, `${field}.stats`);
  const effects = list(row.setEffects, `${field}.setEffects`);
  if (stats.length > 16 || effects.length > 16) {
    throw new TechnicalContractError(`${field} exceeds detail bounds`);
  }
  return {
    uid: uid(row.uid, `${field}.uid`),
    itemId: text(row.itemId, `${field}.itemId`),
    filterId: text(row.filterId, `${field}.filterId`),
    kind: optionalEnum(row.kind, ["module", "core"] as const, `${field}.kind`),
    quality: optionalEnum(
      row.quality,
      ["blue", "purple", "orange"] as const,
      `${field}.quality`,
    ),
    name: text(row.name, `${field}.name`),
    icon: optionalText(row.icon, `${field}.icon`),
    level: uint(row.level, `${field}.level`),
    maxLevel: optionalUint(row.maxLevel, `${field}.maxLevel`),
    locked: flag(row.locked, `${field}.locked`),
    discarded: flag(row.discarded, `${field}.discarded`),
    equippedCharacterUid:
      row.equippedCharacterUid === null
        ? null
        : uid(row.equippedCharacterUid, `${field}.equippedCharacterUid`),
    equippedCharacterId: optionalUint(
      row.equippedCharacterId,
      `${field}.equippedCharacterId`,
    ),
    equippedPlacement:
      row.equippedPlacement === null
        ? null
        : placement(row.equippedPlacement, `${field}.equippedPlacement`),
    stats: stats.map((value, statIndex) => {
      const stat = object(value, `${field}.stats[${statIndex}]`);
      return {
        property: text(stat.property, `${field}.stats[${statIndex}].property`),
        label: text(stat.label, `${field}.stats[${statIndex}].label`),
        value: finite(stat.value, `${field}.stats[${statIndex}].value`),
        percent: flag(stat.percent, `${field}.stats[${statIndex}].percent`),
        main: flag(stat.main, `${field}.stats[${statIndex}].main`),
        unlockLevel: optionalUint(
          stat.unlockLevel,
          `${field}.stats[${statIndex}].unlockLevel`,
        ),
        unlocked: flag(stat.unlocked, `${field}.stats[${statIndex}].unlocked`),
      };
    }),
    setName: optionalText(row.setName, `${field}.setName`),
    setEffects: effects.map((value, effectIndex) => {
      const effect = object(value, `${field}.setEffects[${effectIndex}]`);
      return {
        count: uint(effect.count, `${field}.setEffects[${effectIndex}].count`),
        text: text(effect.text, `${field}.setEffects[${effectIndex}].text`),
      };
    }),
  };
}

function parseOperation(value: unknown): EmptyCurtainOperation {
  const row = object(value, "operation");
  const messageArguments = list(
    row.messageArguments,
    "operation.messageArguments",
  );
  return {
    status: enumValue(
      row.status,
      ["idle", "pending", "success", "error"] as const,
      "operation.status",
    ),
    messageKey: text(row.messageKey, "operation.messageKey"),
    messageArguments: messageArguments.map((value, index) =>
      text(value, `operation.messageArguments[${index}]`),
    ),
  };
}

function uid(value: unknown, field: string): ItemUid {
  const row = object(value, field);
  return {
    slot: uint(row.slot, `${field}.slot`),
    serial: uint(row.serial, `${field}.serial`),
  };
}

function placement(value: unknown, field: string): EmptyCurtainPlacement {
  const row = object(value, field);
  return {
    row: integer(row.row, `${field}.row`),
    column: integer(row.column, `${field}.column`),
  };
}

function object(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TechnicalContractError(`${field} must be an object`);
  }
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

function optionalText(value: unknown, field: string): string | null {
  return value === null ? null : text(value, field);
}

function finite(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new TechnicalContractError(`${field} must be finite`);
  }
  return value;
}

function integer(value: unknown, field: string): number {
  const parsed = finite(value, field);
  if (!Number.isSafeInteger(parsed))
    throw new TechnicalContractError(`${field} must be a safe integer`);
  return parsed;
}

function uint(value: unknown, field: string): number {
  const parsed = integer(value, field);
  if (parsed < 0 || parsed > 0xffffffff)
    throw new TechnicalContractError(
      `${field} must be an unsigned 32-bit integer`,
    );
  return parsed;
}

function optionalUint(value: unknown, field: string): number | null {
  return value === null ? null : uint(value, field);
}

function flag(value: unknown, field: string): boolean {
  if (typeof value !== "boolean")
    throw new TechnicalContractError(`${field} must be a boolean`);
  return value;
}

function decimal(value: unknown, field: string): string {
  const parsed = text(value, field);
  if (!/^(0|[1-9]\d*)$/.test(parsed))
    throw new TechnicalContractError(`${field} must be a decimal string`);
  return parsed;
}

function enumValue<const T extends readonly string[]>(
  value: unknown,
  values: T,
  field: string,
): T[number] {
  if (typeof value !== "string" || !values.includes(value))
    throw new TechnicalContractError(`${field} has an unsupported value`);
  return value as T[number];
}

function optionalEnum<const T extends readonly string[]>(
  value: unknown,
  values: T,
  field: string,
): T[number] | null {
  return value === null ? null : enumValue(value, values, field);
}
