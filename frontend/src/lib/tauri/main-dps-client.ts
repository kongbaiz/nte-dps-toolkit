import { getCurrentWindow } from "@tauri-apps/api/window";

import { invokeCommand, type CommandError } from "@/lib/tauri/command-error";
import type { MainDpsActionResult } from "@/lib/tauri/generated/MainDpsActionResult";
import type { MainDpsEvent } from "@/lib/tauri/generated/MainDpsEvent";
import type { MainDpsResetResult } from "@/lib/tauri/generated/MainDpsResetResult";
import type { MainDpsSnapshot } from "@/lib/tauri/generated/MainDpsSnapshot";
import type { MainDpsDetailFilter } from "@/lib/tauri/main-dps-detail-client";
import { subscribeStream } from "@/lib/tauri/stream-client";

const snapshot = (command: string, arguments_?: Record<string, unknown>) =>
  invokeCommand<MainDpsSnapshot>(command, arguments_);
const command = (name: string, arguments_?: Record<string, unknown>) =>
  invokeCommand<void>(name, arguments_);

export const mainDpsClient = {
  getSnapshot: () => snapshot("get_main_dps_snapshot"),
  startCapture: (replaceCurrent = false) =>
    snapshot("start_main_dps_capture", { replaceCurrent }),
  stopCapture: () => snapshot("stop_main_dps_capture"),
  reset: (confirmed = false) =>
    invokeCommand<MainDpsResetResult>("reset_main_dps_session", { confirmed }),
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
  importReplay: (kind: "json" | "pcapng", replaceCurrent = false) =>
    invokeCommand<MainDpsActionResult>("import_main_dps_replay", {
      kind,
      replaceCurrent,
    }),
  importReplayPath: (path: string, replaceCurrent = false) =>
    invokeCommand<MainDpsActionResult>("import_main_dps_replay_path", {
      path,
      replaceCurrent,
    }),
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
    onError: (error: CommandError) => void,
  ): () => void {
    const close = subscribeStream<MainDpsEvent>({
      streamKind: "mainDps",
      subscribeCommand: "subscribe_main_dps",
      unsubscribeCommand: "unsubscribe_main_dps",
      onEvent: (event) => onSnapshot(event.payload),
      onError,
    });
    return () => void close().catch(onError);
  },
};
