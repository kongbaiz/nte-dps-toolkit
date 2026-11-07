import { useCallback, useEffect, useRef, useState } from "react";

import { settingsClient } from "@/lib/tauri/settings-client";
import type { CaptureSettingsInput } from "@/lib/tauri/generated/CaptureSettingsInput";
import type { GlobalHotkeyAction } from "@/lib/tauri/generated/GlobalHotkeyAction";
import type { HotkeyBindingSnapshot } from "@/lib/tauri/generated/HotkeyBindingSnapshot";
import type { HudModule } from "@/lib/tauri/generated/HudModule";
import type { HudPreset } from "@/lib/tauri/generated/HudPreset";
import type { HudSettingOption } from "@/lib/tauri/generated/HudSettingOption";
import type { InterfaceSettingsInput } from "@/lib/tauri/generated/InterfaceSettingsInput";
import type { LayoutProfile } from "@/lib/tauri/generated/LayoutProfile";
import type { MainDpsDisplayConfig } from "@/lib/tauri/generated/MainDpsDisplayConfig";
import type { SettingsSnapshot } from "@/lib/tauri/generated/SettingsSnapshot";
import type { UpdateComponent } from "@/lib/tauri/generated/UpdateComponent";
import { type CommandError, toCommandError } from "@/lib/tauri/command-error";
import { applySettingsPresentation } from "@/lib/settings-presentation";

import {
  shouldAcceptSettingsGeneration,
  shouldAcceptSettingsRefresh,
  type SettingsPageState,
} from "./settings-view-model";

export function useSettings() {
  const [state, setState] = useState<SettingsPageState>({
    status: "loading",
  });
  const [pendingAction, setPendingAction] = useState<string | null>(null);
  const [mutationError, setMutationError] = useState<CommandError | null>(null);
  const mounted = useRef(true);
  const mutationPending = useRef(false);
  const mutationQueue = useRef<Promise<void>>(Promise.resolve());
  const loadGeneration = useRef(0);
  const acceptedGeneration = useRef<string | null>(null);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const applySnapshot = useCallback(
    (snapshot: SettingsSnapshot, refresh = false) => {
      const accepted = refresh
        ? shouldAcceptSettingsRefresh(
            acceptedGeneration.current,
            snapshot.generation,
          )
        : shouldAcceptSettingsGeneration(
            acceptedGeneration.current,
            snapshot.generation,
          );
      if (!accepted) return;
      acceptedGeneration.current = snapshot.generation;
      applySettingsPresentation(snapshot.interface);
      loadGeneration.current += 1;
      setState({ status: "ready", snapshot });
    },
    [],
  );

  const refresh = useCallback(async () => {
    if (mutationPending.current) return;
    mutationPending.current = true;
    setPendingAction("refresh");
    setMutationError(null);
    try {
      const snapshot = await settingsClient.getSnapshot();
      if (mounted.current) applySnapshot(snapshot, true);
    } catch (error) {
      if (mounted.current) setMutationError(toCommandError(error));
    } finally {
      mutationPending.current = false;
      if (mounted.current) setPendingAction(null);
    }
  }, [applySnapshot]);

  useEffect(() => {
    const unsubscribe = settingsClient.subscribe(
      (snapshot) => {
        if (mounted.current) {
          applySnapshot(snapshot);
        }
      },
      (error) => {
        if (mounted.current) {
          setState({ status: "error", error });
        }
      },
    );
    return () => {
      void unsubscribe().catch((error: unknown) => {
        console.error("settings subscription cleanup failed", error);
      });
    };
  }, [applySnapshot]);

  const mutate = useCallback(
    (
      action: string,
      command: () => Promise<SettingsSnapshot>,
    ): Promise<void> => {
      const run = async () => {
        mutationPending.current = true;
        if (mounted.current) {
          setPendingAction(action);
          setMutationError(null);
        }
        try {
          const snapshot = await command();
          if (mounted.current) {
            applySnapshot(snapshot);
          }
        } catch (error) {
          if (mounted.current) {
            setMutationError(toCommandError(error));
          }
        } finally {
          mutationPending.current = false;
          if (mounted.current) {
            setPendingAction(null);
          }
        }
      };
      const pending = mutationQueue.current.then(run, run);
      mutationQueue.current = pending;
      return pending;
    },
    [applySnapshot],
  );

  return {
    state,
    pendingAction,
    mutationError,
    clearMutationError: () => setMutationError(null),
    refresh,
    setInterface: (settings: InterfaceSettingsInput) =>
      mutate("interface", () => settingsClient.setInterface(settings)),
    setUpdatePreferences: (autoCheck: boolean, autoDownload: boolean) =>
      mutate("update-preferences", () =>
        settingsClient.setUpdatePreferences({ autoCheck, autoDownload }),
      ),
    checkUpdates: () =>
      mutate("update-check", () => settingsClient.checkUpdates()),
    downloadUpdate: (component: UpdateComponent) =>
      mutate(`update-download:${component}`, () =>
        settingsClient.downloadUpdate(component),
      ),
    installUpdate: () =>
      mutate("update-install", () => settingsClient.installUpdate()),
    setCapture: (settings: CaptureSettingsInput) =>
      mutate("capture", () => settingsClient.setCapture(settings)),
    refreshCaptureDevices: () =>
      mutate("capture-devices", () => settingsClient.refreshCaptureDevices()),
    setHotkeysEnabled: (enabled: boolean) =>
      mutate("hotkeys-enabled", () =>
        settingsClient.setHotkeysEnabled(enabled),
      ),
    setHotkeyBinding: (
      action: GlobalHotkeyAction,
      binding: HotkeyBindingSnapshot | null,
    ) =>
      mutate(`hotkey:${action}`, () =>
        settingsClient.setHotkeyBinding(action, binding),
      ),
    setMainDpsDisplay: (settings: MainDpsDisplayConfig) =>
      mutate("main-dps-display", () =>
        settingsClient.setMainDpsDisplay(settings),
      ),
    applyLayoutProfile: (profile: LayoutProfile) =>
      mutate(`layout:${profile}`, () =>
        settingsClient.applyLayoutProfile(profile),
      ),
    openAbyssValues: () =>
      mutate("abyss-values", () => settingsClient.openAbyssValues()),
    importTeamData: (json: string) =>
      mutate("team-import", () => settingsClient.importTeamData(json)),
    importTeamDataFile: async (): Promise<boolean | null> => {
      if (mutationPending.current) return null;
      mutationPending.current = true;
      setPendingAction("team-import");
      setMutationError(null);
      try {
        const result = await settingsClient.importTeamDataFile();
        if (mounted.current) applySnapshot(result.settings);
        return result.performed;
      } catch (error) {
        if (mounted.current) setMutationError(toCommandError(error));
        return null;
      } finally {
        mutationPending.current = false;
        if (mounted.current) setPendingAction(null);
      }
    },
    exportTeamData: async (): Promise<boolean | null> => {
      if (mutationPending.current) {
        return null;
      }
      mutationPending.current = true;
      setPendingAction("team-export");
      setMutationError(null);
      try {
        return await settingsClient.exportTeamData();
      } catch (error) {
        if (mounted.current) {
          setMutationError(toCommandError(error));
        }
        return null;
      } finally {
        mutationPending.current = false;
        if (mounted.current) {
          setPendingAction(null);
        }
      }
    },
    refreshCaptureFiles: () =>
      mutate("capture-files-refresh", () =>
        settingsClient.refreshCaptureFiles(),
      ),
    clearCaptureFiles: () =>
      mutate("capture-files-clear", () => settingsClient.clearCaptureFiles()),
    setHudOption: (option: HudSettingOption, enabled: boolean) =>
      mutate(`option:${option}`, () =>
        settingsClient.setHudOption(option, enabled),
      ),
    applyHudPreset: (preset: HudPreset) =>
      mutate(`preset:${preset}`, () => settingsClient.applyHudPreset(preset)),
    setHudModuleVisibility: (module: HudModule, visible: boolean) =>
      mutate(`module:${module}`, () =>
        settingsClient.setHudModuleVisibility(module, visible),
      ),
    moveHudModule: (
      dragged: HudModule,
      target: HudModule,
      insertAfter: boolean,
    ) =>
      mutate(`move:${dragged}`, () =>
        settingsClient.moveHudModule(dragged, target, insertAfter),
      ),
    setHudWidth: (width: number) =>
      mutate("width", () => settingsClient.setHudWidth(width)),
    setHudAlwaysOnTop: (enabled: boolean) =>
      mutate("always-on-top", () => settingsClient.setHudAlwaysOnTop(enabled)),
    openHudEditor: () =>
      mutate("open-editor", () => settingsClient.openHudEditor()),
  };
}
