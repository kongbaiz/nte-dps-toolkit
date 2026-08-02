import { Channel, invoke } from "@tauri-apps/api/core";

import {
  emptyCurtainError,
  parseEmptyCurtainEvent,
  parseEmptyCurtainFileResult,
  parseEmptyCurtainPositions,
  parseEmptyCurtainSnapshot,
  type CharacterEquipmentAction,
  type EmptyCurtainCommandError,
  type EmptyCurtainFileResult,
  type EmptyCurtainPlacement,
  type EmptyCurtainSnapshot,
  type EquipmentAction,
  type ItemUid,
} from "@/lib/tauri/empty-curtain-contract";
import {
  parseSubscriptionReceipt,
  TechnicalContractError,
} from "@/lib/tauri/technical-contract";

const COMMANDS = {
  getSnapshot: "get_empty_curtain_snapshot",
  subscribe: "subscribe_empty_curtain",
  unsubscribe: "unsubscribe_empty_curtain",
  positions: "get_empty_curtain_positions",
  manageItem: "manage_empty_curtain_item",
  characterAction: "apply_empty_curtain_character_action",
  exportInventory: "export_empty_curtain_inventory",
  exportLoadout: "export_empty_curtain_loadout",
  importLoadout: "import_empty_curtain_loadout",
} as const;

interface EmptyCurtainTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  createChannel(onMessage: (message: unknown) => void): unknown;
}

export interface ManageItemInput {
  item: ItemUid;
  action: EquipmentAction;
  character?: ItemUid;
  position?: EmptyCurtainPlacement;
}

export interface EmptyCurtainClient {
  getSnapshot(): Promise<EmptyCurtainSnapshot>;
  subscribe(
    onSnapshot: (snapshot: EmptyCurtainSnapshot) => void,
    onError: (error: EmptyCurtainCommandError) => void,
  ): () => Promise<void>;
  positions(
    item: ItemUid,
    character: ItemUid,
  ): Promise<EmptyCurtainPlacement[]>;
  manageItem(input: ManageItemInput): Promise<EmptyCurtainSnapshot>;
  characterAction(
    character: ItemUid,
    action: CharacterEquipmentAction,
  ): Promise<EmptyCurtainSnapshot>;
  exportInventory(): Promise<EmptyCurtainFileResult>;
  exportLoadout(character: ItemUid): Promise<EmptyCurtainFileResult>;
  importLoadout(): Promise<EmptyCurtainFileResult>;
}

const tauriTransport: EmptyCurtainTransport = {
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
  createChannel: (onMessage) => {
    const channel = new Channel<unknown>();
    channel.onmessage = onMessage;
    return channel;
  },
};

export function createEmptyCurtainClient(
  transport: EmptyCurtainTransport = tauriTransport,
  createSubscriptionId: () => string = () => crypto.randomUUID(),
): EmptyCurtainClient {
  const command = async <T>(
    name: string,
    arguments_: Record<string, unknown> | undefined,
    parse: (value: unknown) => T,
  ): Promise<T> => {
    try {
      return parse(await transport.invoke(name, arguments_));
    } catch (error) {
      if (error instanceof TechnicalContractError) throw error;
      throw emptyCurtainError(error);
    }
  };
  return {
    getSnapshot: () =>
      command(COMMANDS.getSnapshot, undefined, parseEmptyCurtainSnapshot),
    subscribe: (onSnapshot, onError) => {
      const subscriptionId = createSubscriptionId();
      let closed = false;
      const onEvent = transport.createChannel((message) => {
        if (closed) return;
        try {
          onSnapshot(parseEmptyCurtainEvent(message));
        } catch (error) {
          onError(emptyCurtainError(error));
        }
      });
      const receipt = transport
        .invoke(COMMANDS.subscribe, { subscriptionId, onEvent })
        .then(parseSubscriptionReceipt)
        .catch((error: unknown) => {
          if (!closed) onError(emptyCurtainError(error));
          return undefined;
        });
      return async () => {
        if (closed) return;
        closed = true;
        const activeReceipt = await receipt;
        if (activeReceipt) {
          await transport.invoke(COMMANDS.unsubscribe, {
            subscriptionId: activeReceipt.subscriptionId,
          });
        }
      };
    },
    positions: (item, character) =>
      command(
        COMMANDS.positions,
        { item, character },
        parseEmptyCurtainPositions,
      ),
    manageItem: ({ item, action, character, position }) =>
      command(
        COMMANDS.manageItem,
        {
          item,
          action,
          character: character ?? null,
          row: position?.row ?? null,
          column: position?.column ?? null,
        },
        parseEmptyCurtainSnapshot,
      ),
    characterAction: (character, action) =>
      command(
        COMMANDS.characterAction,
        { character, action },
        parseEmptyCurtainSnapshot,
      ),
    exportInventory: () =>
      command(COMMANDS.exportInventory, undefined, parseEmptyCurtainFileResult),
    exportLoadout: (character) =>
      command(
        COMMANDS.exportLoadout,
        { character },
        parseEmptyCurtainFileResult,
      ),
    importLoadout: () =>
      command(COMMANDS.importLoadout, undefined, parseEmptyCurtainFileResult),
  };
}

export const emptyCurtainClient = createEmptyCurtainClient();
