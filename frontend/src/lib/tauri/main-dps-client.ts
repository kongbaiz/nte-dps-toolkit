import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  parseMainDpsCommandError,
  parseMainDpsEvent,
  parseMainDpsResetResult,
  parseMainDpsSnapshot,
  type MainDpsCommandError,
  type MainDpsSnapshot,
} from "@/lib/tauri/main-dps-contract";
import type { MainDpsDetailFilter } from "@/lib/tauri/main-dps-detail-contract";
import {
  subscribeStream,
  tauriStreamTransport,
} from "@/lib/tauri/stream-client";

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
  startCapture: (replaceCurrent = false) =>
    snapshot("start_main_dps_capture", { replaceCurrent }),
  stopCapture: () => snapshot("stop_main_dps_capture"),
  reset: async (confirmed = false) =>
    parseMainDpsResetResult(
      await command("reset_main_dps_session", { confirmed }),
    ),
  undoReset: (undoToken: string) =>
    snapshot("undo_main_dps_reset", { undoToken }),
  newRound: () => snapshot("start_main_dps_new_round"),
  setOnboardingStep: (step: number) =>
    snapshot("set_main_dps_onboarding_step", { step }),
  finishOnboarding: (hudPreset: "minimal" | "standard" | "detailed") =>
    snapshot("finish_main_dps_onboarding", { hudPreset }),
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
  openConsoleShortcut: (target: "palette" | "packets") =>
    command("open_main_dps_console_shortcut", { target }),
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
  importReplay: async (kind: "json" | "pcapng", replaceCurrent = false) => {
    const result = await command("import_main_dps_replay", {
      kind,
      replaceCurrent,
    });
    const source = result as { performed?: unknown; snapshot?: unknown };
    return {
      performed: source.performed === true,
      snapshot: parseMainDpsSnapshot(source.snapshot),
    };
  },
  importReplayPath: async (path: string, replaceCurrent = false) => {
    const result = await command("import_main_dps_replay_path", {
      path,
      replaceCurrent,
    });
    const source = result as { performed?: unknown; snapshot?: unknown };
    return {
      performed: source.performed === true,
      snapshot: parseMainDpsSnapshot(source.snapshot),
    };
  },
  subscribeReplayDrops(onPath: (path: string) => void) {
    return getCurrentWindow().onDragDropEvent((event) => {
      if (event.payload.type !== "drop") return;
      const path = event.payload.paths.find((candidate) =>
        /\.(?:pcapng|json)$/i.test(candidate),
      );
      if (path !== undefined) onPath(path);
    });
  },
  subscribe(
    onSnapshot: (value: MainDpsSnapshot) => void,
    onError: (error: MainDpsCommandError) => void,
  ): () => void {
    const subscriptionId = crypto.randomUUID();
    const close = subscribeStream({
      transport: tauriStreamTransport,
      streamKind: "mainDps",
      subscriptionId,
      subscribeCommand: "subscribe_main_dps",
      unsubscribeCommand: "unsubscribe_main_dps",
      parseEvent: parseMainDpsEvent,
      onEvent: onSnapshot,
      onError: (error) => onError(parseMainDpsCommandError(error)),
    });
    return () => {
      void close().catch((error: unknown) =>
        onError(parseMainDpsCommandError(error)),
      );
    };
  },
};
