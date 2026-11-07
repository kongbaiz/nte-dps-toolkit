import { invokeCommand, type CommandError } from "@/lib/tauri/command-error";
import type { EmptyCurtainEvent } from "@/lib/tauri/generated/EmptyCurtainEvent";
import type { EmptyCurtainFileResult } from "@/lib/tauri/generated/EmptyCurtainFileResult";
import type { EmptyCurtainPositionSnapshot } from "@/lib/tauri/generated/EmptyCurtainPositionSnapshot";
import type { EmptyCurtainSnapshot } from "@/lib/tauri/generated/EmptyCurtainSnapshot";
import type { ItemUidSnapshot } from "@/lib/tauri/generated/ItemUidSnapshot";
import { subscribeStream } from "@/lib/tauri/stream-client";

export type EquipmentAction =
  "lock" | "unlock" | "discard" | "restore" | "unequip" | "equip";
export type CharacterEquipmentAction = "unequip-all" | "one-click";

export interface ManageItemInput {
  item: ItemUidSnapshot;
  action: EquipmentAction;
  character?: ItemUidSnapshot;
  position?: EmptyCurtainPositionSnapshot;
}

export function itemUidKey(value: ItemUidSnapshot): string {
  return `${value.slot}:${value.serial}`;
}

const fileAction = (command: string, arguments_?: Record<string, unknown>) =>
  invokeCommand<EmptyCurtainFileResult>(command, arguments_);

export const emptyCurtainClient = {
  getSnapshot: () =>
    invokeCommand<EmptyCurtainSnapshot>("get_empty_curtain_snapshot"),
  subscribe: (
    onSnapshot: (snapshot: EmptyCurtainSnapshot) => void,
    onError: (error: CommandError) => void,
  ) =>
    subscribeStream<EmptyCurtainEvent>({
      streamKind: "emptyCurtain",
      subscribeCommand: "subscribe_empty_curtain",
      unsubscribeCommand: "unsubscribe_empty_curtain",
      onEvent: (event) => onSnapshot(event.payload),
      onError,
    }),
  positions: (item: ItemUidSnapshot, character: ItemUidSnapshot) =>
    invokeCommand<EmptyCurtainPositionSnapshot[]>(
      "get_empty_curtain_positions",
      { item, character },
    ),
  manageItem: ({ item, action, character, position }: ManageItemInput) =>
    invokeCommand<EmptyCurtainSnapshot>("manage_empty_curtain_item", {
      item,
      action,
      character: character ?? null,
      row: position?.row ?? null,
      column: position?.column ?? null,
    }),
  characterAction: (
    character: ItemUidSnapshot,
    action: CharacterEquipmentAction,
  ) =>
    invokeCommand<EmptyCurtainSnapshot>(
      "apply_empty_curtain_character_action",
      { character, action },
    ),
  exportInventory: () => fileAction("export_empty_curtain_inventory"),
  exportLoadout: (character: ItemUidSnapshot) =>
    fileAction("export_empty_curtain_loadout", { character }),
  importLoadout: () => fileAction("import_empty_curtain_loadout"),
};
