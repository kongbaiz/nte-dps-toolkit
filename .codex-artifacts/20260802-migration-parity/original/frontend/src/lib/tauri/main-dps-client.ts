import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  parseMainDpsCommandError,
  parseMainDpsEvent,
  parseMainDpsSnapshot,
  type MainDpsCommandError,
  type MainDpsSnapshot,
} from "@/lib/tauri/main-dps-contract";
import type { MainDpsDetailFilter } from "@/lib/tauri/main-dps-detail-contract";

const command = (name: string, arguments_?: Record<string, unknown>) =>
  invoke<unknown>(name, arguments_);
const snapshot = async (name: string, arguments_?: Record<string, unknown>) =>
  parseMainDpsSnapshot(await command(name, arguments_));

interface DraggableWindow {
  startDragging(): Promise<void>;
}

type CurrentWindowProvider = () => DraggableWindow;

export function startMainDpsWindowDragging(
  currentWindow: CurrentWindowProvider = getCurrentWindow,
): Promise<void> {
  return currentWindow().startDragging();
}

export const mainDpsClient = {
  getSnapshot: () => snapshot("get_main_dps_snapshot"),
  startCapture: () => snapshot("start_main_dps_capture"),
  stopCapture: () => snapshot("stop_main_dps_capture"),
  reset: () => snapshot("reset_main_dps_session"),
  newRound: () => snapshot("start_main_dps_new_round"),
  setPaused: (paused: boolean) => snapshot("set_main_dps_paused", { paused }),
  selectRound: (recordId: string | null) =>
    snapshot("select_main_dps_round", { recordId }),
  selectAbyssHalf: (half: "all" | "first" | "second") =>
    snapshot("select_main_dps_abyss_half", { half }),
  setAlwaysOnTop: (enabled: boolean) =>
    snapshot("set_main_dps_always_on_top", { enabled }),
  setAppearance: (darkMode: boolean, opacity: number) =>
    snapshot("set_main_dps_appearance", { darkMode, opacity }),
  openConsole: () => command("open_main_dps_console"),
  openHud: () => command("open_main_dps_hud"),
  openCharacterDetails: (characterId: number) =>
    command("open_main_dps_character_details", { characterId }),
  openTeamDetails: (filter: Exclude<MainDpsDetailFilter, "qteType"> = "all") =>
    command("open_main_dps_team_details", { filter }),
  setPassthrough: (enabled: boolean) =>
    snapshot("set_main_dps_passthrough", { enabled }),
  minimize: () => command("minimize_main_dps_window"),
  toggleMaximized: () => command("toggle_main_dps_maximized"),
  close: () => command("close_main_dps_window"),
  importReplay: async (kind: "json" | "pcapng") => {
    const result = await command("import_main_dps_replay", { kind });
    const source = result as { performed?: unknown; snapshot?: unknown };
    return {
      performed: source.performed === true,
      snapshot: parseMainDpsSnapshot(source.snapshot),
    };
  },
  subscribe(
    onSnapshot: (value: MainDpsSnapshot) => void,
    onError: (error: MainDpsCommandError) => void,
  ) {
    const subscriptionId = crypto.randomUUID();
    const channel = new Channel<unknown>();
    channel.onmessage = (message) => {
      try {
        onSnapshot(parseMainDpsEvent(message));
      } catch (error) {
        onError(parseMainDpsCommandError(error));
      }
    };
    void command("subscribe_main_dps", {
      subscriptionId,
      onEvent: channel,
    }).catch((error) => onError(parseMainDpsCommandError(error)));
    return () =>
      command("unsubscribe_main_dps", { subscriptionId })
        .then(() => undefined)
        .catch(() => undefined);
  },
};
