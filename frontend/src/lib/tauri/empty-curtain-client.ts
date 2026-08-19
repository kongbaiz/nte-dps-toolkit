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
import { TechnicalContractError } from "@/lib/tauri/technical-contract";
import {
  subscribeAckedStream,
  tauriAckedStreamTransport,
} from "@/lib/tauri/stream-client";

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

const tauriTransport: EmptyCurtainTransport = tauriAckedStreamTransport;

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
      return subscribeAckedStream({
        transport,
        streamKind: "emptyCurtain",
        subscriptionId,
        subscribeCommand: COMMANDS.subscribe,
        unsubscribeCommand: COMMANDS.unsubscribe,
        parseEvent: parseEmptyCurtainEvent,
        onEvent: onSnapshot,
        onError: (error) => onError(emptyCurtainError(error)),
      });
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
