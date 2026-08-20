import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const EMPTY_CURTAIN_CONTRACT_VERSION = 2;
export const EMPTY_CURTAIN_MAX_CHARACTERS = 64;
export const EMPTY_CURTAIN_MAX_ITEMS = 4096;
export const EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM = 16;
export const EMPTY_CURTAIN_MAX_TEXT_BYTES = 256;
export const EMPTY_CURTAIN_MAX_ICON_BYTES = 1024;
export const EMPTY_CURTAIN_MAX_TOTAL_DETAILS = 20_000;
export const EMPTY_CURTAIN_MAX_PROJECTED_TEXT_BYTES = 1024 * 1024;

const UTF8_ENCODER = new TextEncoder();

const {
  array: list,
  boolean: flag,
  boundedUtf8String,
  boundedUtf8StringAllowEmpty,
  decimalString: decimal,
  enumValue,
  finiteNumber: finite,
  integer,
  nullableEnumValue: optionalEnum,
  nullableBoundedUtf8StringAllowEmpty: optionalBoundedUtf8Text,
  nullableUnsigned32: optionalUint,
  record: object,
  unsigned32: uint,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

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
  const parsedCharacters = characters.map((value, index) => {
    const row = object(value, `characters[${index}]`);
    return {
      uid: uid(row.uid, `characters[${index}].uid`),
      characterId: uint(row.characterId, `characters[${index}].characterId`),
      name: boundedUtf8String(
        row.name,
        `characters[${index}].name`,
        EMPTY_CURTAIN_MAX_TEXT_BYTES,
      ),
    };
  });
  const parsedItems = items.map(parseItem);
  const detailRows = parsedItems.reduce(
    (total, item) => total + item.stats.length + item.setEffects.length,
    0,
  );
  if (detailRows > EMPTY_CURTAIN_MAX_TOTAL_DETAILS) {
    throw new TechnicalContractError("items exceeds aggregate detail bounds");
  }
  const projectedTextBytes =
    parsedCharacters.reduce(
      (total, character) => total + utf8Bytes(character.name),
      0,
    ) + parsedItems.reduce((total, item) => total + itemTextBytes(item), 0);
  if (projectedTextBytes > EMPTY_CURTAIN_MAX_PROJECTED_TEXT_BYTES) {
    throw new TechnicalContractError("items exceeds aggregate text bounds");
  }
  return {
    contractVersion,
    generation: decimal(data.generation, "generation"),
    observedAtUnixMs: decimal(data.observedAtUnixMs, "observedAtUnixMs"),
    hasData: flag(data.hasData, "hasData"),
    complete: flag(data.complete, "complete"),
    characters: parsedCharacters,
    items: parsedItems,
    operation: parseOperation(data.operation),
  };
}

function utf8Bytes(value: string): number {
  return UTF8_ENCODER.encode(value).byteLength;
}

function itemTextBytes(item: EmptyCurtainItem): number {
  let total =
    utf8Bytes(item.itemId) +
    utf8Bytes(item.filterId) +
    (item.quality === null ? 0 : utf8Bytes(item.quality)) +
    utf8Bytes(item.name) +
    (item.icon === null ? 0 : utf8Bytes(item.icon)) +
    (item.setName === null ? 0 : utf8Bytes(item.setName));
  for (const stat of item.stats) {
    total += utf8Bytes(stat.property) + utf8Bytes(stat.label);
  }
  for (const effect of item.setEffects) {
    total += utf8Bytes(effect.text);
  }
  return total;
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
  if (
    stats.length > EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM ||
    effects.length > EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM
  ) {
    throw new TechnicalContractError(`${field} exceeds detail bounds`);
  }
  return {
    uid: uid(row.uid, `${field}.uid`),
    itemId: boundedUtf8String(
      row.itemId,
      `${field}.itemId`,
      EMPTY_CURTAIN_MAX_TEXT_BYTES,
    ),
    filterId: boundedUtf8String(
      row.filterId,
      `${field}.filterId`,
      EMPTY_CURTAIN_MAX_TEXT_BYTES,
    ),
    kind: optionalEnum(row.kind, ["module", "core"] as const, `${field}.kind`),
    quality: optionalEnum(
      row.quality,
      ["blue", "purple", "orange"] as const,
      `${field}.quality`,
    ),
    name: boundedUtf8String(
      row.name,
      `${field}.name`,
      EMPTY_CURTAIN_MAX_TEXT_BYTES,
    ),
    icon: optionalBoundedUtf8Text(
      row.icon,
      `${field}.icon`,
      EMPTY_CURTAIN_MAX_ICON_BYTES,
    ),
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
        property: boundedUtf8StringAllowEmpty(
          stat.property,
          `${field}.stats[${statIndex}].property`,
          EMPTY_CURTAIN_MAX_TEXT_BYTES,
        ),
        label: boundedUtf8StringAllowEmpty(
          stat.label,
          `${field}.stats[${statIndex}].label`,
          EMPTY_CURTAIN_MAX_TEXT_BYTES,
        ),
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
    setName: optionalBoundedUtf8Text(
      row.setName,
      `${field}.setName`,
      EMPTY_CURTAIN_MAX_TEXT_BYTES,
    ),
    setEffects: effects.map((value, effectIndex) => {
      const effect = object(value, `${field}.setEffects[${effectIndex}]`);
      return {
        count: uint(effect.count, `${field}.setEffects[${effectIndex}].count`),
        text: boundedUtf8StringAllowEmpty(
          effect.text,
          `${field}.setEffects[${effectIndex}].text`,
          EMPTY_CURTAIN_MAX_TEXT_BYTES,
        ),
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
  if (messageArguments.length > EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM) {
    throw new TechnicalContractError(
      "operation.messageArguments exceeds bounds",
    );
  }
  return {
    status: enumValue(
      row.status,
      ["idle", "pending", "success", "error"] as const,
      "operation.status",
    ),
    messageKey: boundedUtf8String(
      row.messageKey,
      "operation.messageKey",
      EMPTY_CURTAIN_MAX_TEXT_BYTES,
    ),
    messageArguments: messageArguments.map((value, index) =>
      boundedUtf8StringAllowEmpty(
        value,
        `operation.messageArguments[${index}]`,
        EMPTY_CURTAIN_MAX_TEXT_BYTES,
      ),
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
