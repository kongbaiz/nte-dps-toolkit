import { invokeCommand, type CommandError } from "@/lib/tauri/command-error";
import type { HudModule } from "@/lib/tauri/generated/HudModule";
import type { TechnicalEvent } from "@/lib/tauri/generated/TechnicalEvent";
import type { TechnicalSnapshot } from "@/lib/tauri/generated/TechnicalSnapshot";
import { subscribeStream } from "@/lib/tauri/stream-client";

const snapshot = (command: string, arguments_?: Record<string, unknown>) =>
  invokeCommand<TechnicalSnapshot>(command, arguments_);

export const technicalClient = {
  getSnapshot: () => snapshot("get_technical_snapshot"),
  moveModule: (dragged: HudModule, target: HudModule, insertAfter: boolean) =>
    snapshot("move_hud_module", { dragged, target, insertAfter }),
  resetSession: () => snapshot("reset_hud_session"),
  setAlwaysOnTop: (enabled: boolean) =>
    snapshot("set_hud_always_on_top", { enabled }),
  setModuleVisibility: (module: HudModule, visible: boolean) =>
    snapshot("set_hud_module_visibility", { module, visible }),
  setPassthrough: (enabled: boolean) =>
    snapshot("set_hud_passthrough", { enabled }),
  setWidth: (width: number) => snapshot("set_hud_width", { width }),
  startCapture: () => snapshot("start_hud_capture"),
  stopCapture: () => snapshot("stop_hud_capture"),
  subscribe: (
    onSnapshot: (snapshot: TechnicalSnapshot) => void,
    onError: (error: CommandError) => void,
  ) =>
    subscribeStream<TechnicalEvent>({
      streamKind: "technical",
      subscribeCommand: "subscribe_technical_state",
      unsubscribeCommand: "unsubscribe_technical_state",
      onEvent: (event) => onSnapshot(event.payload),
      onError,
    }),
};
