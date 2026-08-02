import { useCallback, useEffect, useRef, useState } from "react";

import {
  settingsClient,
  settingsError,
  type SettingsClient,
} from "@/lib/tauri/settings-client";
import type {
  CaptureSettingsInput,
  GlobalHotkeyActionId,
  HotkeyBinding,
  HudPresetId,
  HudSettingOptionId,
  InterfaceSettingsInput,
  LayoutProfileId,
  SettingsCommandError,
  SettingsSnapshot,
  UpdateComponentId,
} from "@/lib/tauri/settings-contract";
import { applySettingsPresentation } from "@/lib/settings-presentation";
import type { HudModuleId } from "@/lib/tauri/technical-contract";

import {
  shouldAcceptSettingsGeneration,
  shouldAcceptSettingsRefresh,
  type SettingsPageState,
} from "./settings-view-model";

export function useSettings(client: SettingsClient = settingsClient) {
  const [state, setState] = useState<SettingsPageState>({
    status: "loading",
  });
  const [pendingAction, setPendingAction] = useState<string | null>(null);
  const [mutationError, setMutationError] =
    useState<SettingsCommandError | null>(null);
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
      const snapshot = await client.getSnapshot();
      if (mounted.current) applySnapshot(snapshot, true);
    } catch (error) {
      if (mounted.current) setMutationError(settingsError(error));
    } finally {
      mutationPending.current = false;
      if (mounted.current) setPendingAction(null);
    }
  }, [applySnapshot, client]);

  useEffect(() => {
    const unsubscribe = client.subscribe(
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
  }, [applySnapshot, client]);

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
            setMutationError(settingsError(error));
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
      mutate("interface", () => client.setInterface(settings)),
    setUpdatePreferences: (autoCheck: boolean, autoDownload: boolean) =>
      mutate("update-preferences", () =>
        client.setUpdatePreferences({ autoCheck, autoDownload }),
      ),
    checkUpdates: () => mutate("update-check", () => client.checkUpdates()),
    downloadUpdate: (component: UpdateComponentId) =>
      mutate(`update-download:${component}`, () =>
        client.downloadUpdate(component),
      ),
    installUpdate: () => mutate("update-install", () => client.installUpdate()),
    setCapture: (settings: CaptureSettingsInput) =>
      mutate("capture", () => client.setCapture(settings)),
    refreshCaptureDevices: () =>
      mutate("capture-devices", () => client.refreshCaptureDevices()),
    setHotkeysEnabled: (enabled: boolean) =>
      mutate("hotkeys-enabled", () => client.setHotkeysEnabled(enabled)),
    setHotkeyBinding: (
      action: GlobalHotkeyActionId,
      binding: HotkeyBinding | null,
    ) =>
      mutate(`hotkey:${action}`, () =>
        client.setHotkeyBinding(action, binding),
      ),
    applyLayoutProfile: (profile: LayoutProfileId) =>
      mutate(`layout:${profile}`, () => client.applyLayoutProfile(profile)),
    openAbyssValues: () =>
      mutate("abyss-values", () => client.openAbyssValues()),
    importTeamData: (json: string) =>
      mutate("team-import", () => client.importTeamData(json)),
    importTeamDataFile: async (): Promise<boolean | null> => {
      if (mutationPending.current) return null;
      mutationPending.current = true;
      setPendingAction("team-import");
      setMutationError(null);
      try {
        const result = await client.importTeamDataFile();
        if (mounted.current) applySnapshot(result.settings);
        return result.performed;
      } catch (error) {
        if (mounted.current) setMutationError(settingsError(error));
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
        return await client.exportTeamData();
      } catch (error) {
        if (mounted.current) {
          setMutationError(settingsError(error));
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
      mutate("capture-files-refresh", () => client.refreshCaptureFiles()),
    clearCaptureFiles: () =>
      mutate("capture-files-clear", () => client.clearCaptureFiles()),
    setHudOption: (option: HudSettingOptionId, enabled: boolean) =>
      mutate(`option:${option}`, () => client.setHudOption(option, enabled)),
    applyHudPreset: (preset: HudPresetId) =>
      mutate(`preset:${preset}`, () => client.applyHudPreset(preset)),
    setHudModuleVisibility: (module: HudModuleId, visible: boolean) =>
      mutate(`module:${module}`, () =>
        client.setHudModuleVisibility(module, visible),
      ),
    moveHudModule: (
      dragged: HudModuleId,
      target: HudModuleId,
      insertAfter: boolean,
    ) =>
      mutate(`move:${dragged}`, () =>
        client.moveHudModule(dragged, target, insertAfter),
      ),
    setHudWidth: (width: number) =>
      mutate("width", () => client.setHudWidth(width)),
    setHudAlwaysOnTop: (enabled: boolean) =>
      mutate("always-on-top", () => client.setHudAlwaysOnTop(enabled)),
    openHudEditor: () => mutate("open-editor", () => client.openHudEditor()),
  };
}
