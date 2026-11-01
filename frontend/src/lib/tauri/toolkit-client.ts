import { invokeCommand } from "@/lib/tauri/command-error";
import type { DataMode } from "@/lib/tauri/generated/DataMode";
import type { HostLaunchResult } from "@/lib/tauri/generated/HostLaunchResult";
import type { ModStudioLoadingMethod } from "@/lib/tauri/generated/ModStudioLoadingMethod";
import type { PluginPanel } from "@/lib/tauri/generated/PluginPanel";

const panel = (command: string, arguments_?: Record<string, unknown>) =>
  invokeCommand<PluginPanel>(command, arguments_);

export const toolkitClient = {
  snapshot: () => panel("get_plugin_panel"),
  setMode: (mode: DataMode, acknowledgeRisk: boolean) =>
    panel("set_data_mode", { mode, acknowledgeRisk }),
  setLoadingMethod: (method: ModStudioLoadingMethod) =>
    panel("set_host_loading_method", { method }),
  selectGameDirectory: () => panel("select_host_game_directory"),
  clearGameDirectory: () => panel("clear_host_game_directory"),
  uninstallProxy: (confirmed: boolean) =>
    panel("uninstall_proxy_host", { confirmed }),
  launchHost: () => invokeCommand<HostLaunchResult>("launch_plugin_host"),
  control: (
    action: "enable" | "disable",
    file: string | null = null,
    value: number | null = null,
  ) => invokeCommand<void>("control_plugin", { action, file, value }),
};
