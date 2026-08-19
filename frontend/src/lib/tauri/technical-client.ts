import {
  parseTechnicalCommandError,
  parseTechnicalEvent,
  parseTechnicalSnapshot,
  TechnicalContractError,
  type HudModuleId,
  type TechnicalCommandError,
  type TechnicalSnapshot,
} from "@/lib/tauri/technical-contract";
import {
  subscribeAckedStream,
  tauriAckedStreamTransport,
} from "@/lib/tauri/stream-client";

const COMMANDS = {
  getSnapshot: "get_technical_snapshot",
  moveModule: "move_hud_module",
  resetSession: "reset_hud_session",
  setAlwaysOnTop: "set_hud_always_on_top",
  setModuleVisibility: "set_hud_module_visibility",
  setPassthrough: "set_hud_passthrough",
  setWidth: "set_hud_width",
  startCapture: "start_hud_capture",
  stopCapture: "stop_hud_capture",
  subscribe: "subscribe_technical_state",
  unsubscribe: "unsubscribe_technical_state",
} as const;

interface TechnicalTransport {
  invoke(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<unknown>;
  createChannel(onMessage: (message: unknown) => void): unknown;
}

export interface TechnicalClient {
  getSnapshot(): Promise<TechnicalSnapshot>;
  moveModule(
    dragged: HudModuleId,
    target: HudModuleId,
    insertAfter: boolean,
  ): Promise<TechnicalSnapshot>;
  resetSession(): Promise<TechnicalSnapshot>;
  setAlwaysOnTop(enabled: boolean): Promise<TechnicalSnapshot>;
  setModuleVisibility(
    module: HudModuleId,
    visible: boolean,
  ): Promise<TechnicalSnapshot>;
  setPassthrough(enabled: boolean): Promise<TechnicalSnapshot>;
  setWidth(width: number): Promise<TechnicalSnapshot>;
  startCapture(): Promise<TechnicalSnapshot>;
  stopCapture(): Promise<TechnicalSnapshot>;
  subscribe(
    onSnapshot: (snapshot: TechnicalSnapshot) => void,
    onError: (error: TechnicalCommandError) => void,
  ): () => Promise<void>;
}

const tauriTransport: TechnicalTransport = tauriAckedStreamTransport;

export function createTechnicalClient(
  transport: TechnicalTransport = tauriTransport,
  createSubscriptionId: () => string = () => crypto.randomUUID(),
): TechnicalClient {
  async function snapshotCommand(
    command: string,
    arguments_?: Record<string, unknown>,
  ): Promise<TechnicalSnapshot> {
    try {
      return parseTechnicalSnapshot(
        await transport.invoke(command, arguments_),
      );
    } catch (error) {
      if (error instanceof TechnicalContractError) {
        throw error;
      }
      throw parseTechnicalCommandError(error);
    }
  }

  return {
    getSnapshot: () => snapshotCommand(COMMANDS.getSnapshot),
    moveModule: (dragged, target, insertAfter) =>
      snapshotCommand(COMMANDS.moveModule, {
        dragged,
        target,
        insertAfter,
      }),
    resetSession: () => snapshotCommand(COMMANDS.resetSession),
    setAlwaysOnTop: (enabled) =>
      snapshotCommand(COMMANDS.setAlwaysOnTop, { enabled }),
    setModuleVisibility: (module, visible) =>
      snapshotCommand(COMMANDS.setModuleVisibility, { module, visible }),
    setPassthrough: (enabled) =>
      snapshotCommand(COMMANDS.setPassthrough, { enabled }),
    setWidth: (width) => snapshotCommand(COMMANDS.setWidth, { width }),
    startCapture: () => snapshotCommand(COMMANDS.startCapture),
    stopCapture: () => snapshotCommand(COMMANDS.stopCapture),
    subscribe: (onSnapshot, onError) => {
      const subscriptionId = createSubscriptionId();
      return subscribeAckedStream({
        transport,
        streamKind: "technical",
        subscriptionId,
        subscribeCommand: COMMANDS.subscribe,
        unsubscribeCommand: COMMANDS.unsubscribe,
        parseEvent: (value) => parseTechnicalEvent(value).payload,
        onEvent: onSnapshot,
        onError: (error) => onError(parseTechnicalCommandError(error)),
      });
    },
  };
}

export const technicalClient = createTechnicalClient();
