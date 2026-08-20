import { useCallback, useEffect, useRef, useState } from "react";

import {
  modStudioClient,
  type ModStudioClient,
} from "@/lib/tauri/mod-studio-client";
import {
  parseModStudioCommandError,
  type ModLoadingMethod,
  type ModStudioDeploymentSnapshot,
  type ModStudioGameRegion,
  type ModLoaderRuntimeSnapshot,
  type ModStudioSdkSchemaSnapshot,
} from "@/lib/tauri/mod-studio-contract";

import {
  acceptDocument,
  acceptEnabledWorkspace,
  acceptRuntimeEvent,
  acceptSavedDocument,
  acceptSavedSource,
  acceptWorkspace,
  editSourceBuffer,
  isSourceBufferDirty,
  INITIAL_MOD_STUDIO_RUNTIME_STATE,
  mergeLoadedSource,
  rejectDocument,
  revertSourceBuffer,
  selectDocument,
  selectedIdOf,
  type ModStudioSourceBuffer,
  type ModStudioPageState,
} from "./mod-studio-view-model";

export type ModStudioSaveState =
  | { status: "idle" }
  | { status: "saving" }
  | { status: "saved" }
  | {
      status: "error";
      error: ReturnType<typeof parseModStudioCommandError>;
    };

export type ModStudioEnableState =
  | { status: "idle" }
  | { status: "saving" }
  | {
      status: "error";
      error: ReturnType<typeof parseModStudioCommandError>;
    };

export type ModStudioSdkState =
  | { status: "loading" }
  | { status: "ready"; schema: ModStudioSdkSchemaSnapshot }
  | {
      status: "error";
      error: ReturnType<typeof parseModStudioCommandError>;
    };

export type ModLoaderControlState =
  | { status: "loading" }
  | {
      status: "ready";
      snapshot: ModLoaderRuntimeSnapshot;
      operation: "idle" | "updating";
    }
  | {
      status: "error";
      error: ReturnType<typeof parseModStudioCommandError>;
    };

export type ModStudioDeploymentState =
  | { status: "loading" }
  | {
      status: "ready";
      snapshot: ModStudioDeploymentSnapshot;
      operation: "idle" | "choosing" | "updating";
    }
  | {
      status: "error";
      error: ReturnType<typeof parseModStudioCommandError>;
    };

export type { ModLoadingMethod } from "@/lib/tauri/mod-studio-contract";

export type ModStudioPreferenceState =
  | { status: "loading" }
  | {
      status: "ready";
      riskAcknowledged: boolean;
      operation: "idle" | "updating";
    }
  | {
      status: "error";
      error: ReturnType<typeof parseModStudioCommandError>;
    };

export type ModStudioActionState =
  | { status: "idle" }
  | { status: "working" }
  | { status: "done" }
  | {
      status: "error";
      error: ReturnType<typeof parseModStudioCommandError>;
    };

export type ModStudioDeleteState =
  | { status: "idle" }
  | { status: "deleting" }
  | {
      status: "error";
      error: ReturnType<typeof parseModStudioCommandError>;
    };

export function useModStudio(client: ModStudioClient = modStudioClient) {
  const [state, setState] = useState<ModStudioPageState>({
    status: "loading",
  });
  const workspaceRequest = useRef(0);
  const savingIds = useRef(new Set<string>());
  const enablingIds = useRef(new Set<string>());
  const deletingIds = useRef(new Set<string>());
  const [buffers, setBuffers] = useState<Record<string, ModStudioSourceBuffer>>(
    {},
  );
  const [saveStates, setSaveStates] = useState<
    Record<string, ModStudioSaveState>
  >({});
  const [enableStates, setEnableStates] = useState<
    Record<string, ModStudioEnableState>
  >({});
  const [deleteStates, setDeleteStates] = useState<
    Record<string, ModStudioDeleteState>
  >({});
  const [runtimeState, setRuntimeState] = useState(
    INITIAL_MOD_STUDIO_RUNTIME_STATE,
  );
  const [sdkState, setSdkState] = useState<ModStudioSdkState>({
    status: "loading",
  });
  const [loaderState, setLoaderState] = useState<ModLoaderControlState>({
    status: "loading",
  });
  const [loadingMethod, setLoadingMethod] = useState<ModLoadingMethod>("proxy");
  const [preferenceState, setPreferenceState] =
    useState<ModStudioPreferenceState>({ status: "loading" });
  const [selectedRegion, setSelectedRegionState] =
    useState<ModStudioGameRegion>("china");
  const [manualDirectories, setManualDirectories] = useState<
    Partial<Record<ModStudioGameRegion, string>>
  >({});
  const [deploymentState, setDeploymentState] =
    useState<ModStudioDeploymentState>({ status: "loading" });
  const [loaderDirectoryState, setLoaderDirectoryState] =
    useState<ModStudioActionState>({ status: "idle" });
  const [createState, setCreateState] = useState<ModStudioActionState>({
    status: "idle",
  });
  const [folderState, setFolderState] = useState<ModStudioActionState>({
    status: "idle",
  });
  const loaderRequest = useRef(0);
  const deploymentRequest = useRef(0);
  const preferenceRequest = useRef(0);
  const initialRegionResolved = useRef(false);
  const selectedId = useRef<string | null>(null);
  const currentSelectedId = selectedIdOf(state);
  if (currentSelectedId !== null) {
    selectedId.current = currentSelectedId;
  }

  const refresh = useCallback(async () => {
    const request = ++workspaceRequest.current;
    const preferredId = selectedId.current;
    setState({ status: "loading" });
    try {
      const workspace = await client.getWorkspace();
      if (request === workspaceRequest.current) {
        setState(acceptWorkspace(workspace, preferredId));
      }
    } catch (error) {
      if (request === workspaceRequest.current) {
        setState({
          status: "error",
          error: parseModStudioCommandError(error),
        });
      }
    }
  }, [client]);

  const refreshLoader = useCallback(async () => {
    const request = ++loaderRequest.current;
    setLoaderState({ status: "loading" });
    try {
      const snapshot = await client.getLoaderRuntime();
      if (request !== loaderRequest.current) return;
      setLoaderState({ status: "ready", snapshot, operation: "idle" });
    } catch (error) {
      if (request !== loaderRequest.current) return;
      setLoaderState({
        status: "error",
        error: parseModStudioCommandError(error),
      });
    }
  }, [client]);

  const refreshDeployment = useCallback(async () => {
    const request = ++deploymentRequest.current;
    setDeploymentState({ status: "loading" });
    try {
      const preference = await client.getGameDirectory(selectedRegion);
      if (request !== deploymentRequest.current) return;
      const directory = preference.path;
      setManualDirectories((current) => {
        const next = { ...current };
        if (directory === null) {
          delete next[selectedRegion];
        } else {
          next[selectedRegion] = directory;
        }
        return next;
      });
      const snapshot = await client.getDeployment(
        directory === null ? null : selectedRegion,
        directory,
      );
      if (request !== deploymentRequest.current) return;
      if (!initialRegionResolved.current) {
        initialRegionResolved.current = true;
        const detected =
          snapshot.games.find((game) => game.installed) ?? snapshot.games[0];
        if (detected !== undefined) {
          setSelectedRegionState(detected.region);
        }
      }
      setDeploymentState({ status: "ready", snapshot, operation: "idle" });
    } catch (error) {
      if (request !== deploymentRequest.current) return;
      setDeploymentState({
        status: "error",
        error: parseModStudioCommandError(error),
      });
    }
  }, [client, selectedRegion]);

  const refreshPreferences = useCallback(async () => {
    const request = ++preferenceRequest.current;
    setPreferenceState({ status: "loading" });
    try {
      const preference = await client.getLoadingMethod();
      if (request !== preferenceRequest.current) return;
      setLoadingMethod(preference.method);
      setPreferenceState({
        status: "ready",
        riskAcknowledged: preference.riskAcknowledged,
        operation: "idle",
      });
    } catch (error) {
      if (request !== preferenceRequest.current) return;
      setPreferenceState({
        status: "error",
        error: parseModStudioCommandError(error),
      });
    }
  }, [client]);

  const refreshAll = useCallback(async () => {
    await Promise.all([
      refresh(),
      refreshDeployment(),
      refreshLoader(),
      refreshPreferences(),
    ]);
  }, [refresh, refreshDeployment, refreshLoader, refreshPreferences]);

  const chooseDocument = useCallback((id: string) => {
    setState((current) => selectDocument(current, id));
  }, []);

  useEffect(() => {
    void refresh();
    return () => {
      workspaceRequest.current += 1;
    };
  }, [refresh]);

  useEffect(() => {
    void refreshLoader();
    return () => {
      loaderRequest.current += 1;
    };
  }, [refreshLoader]);

  useEffect(() => {
    void refreshDeployment();
    return () => {
      deploymentRequest.current += 1;
    };
  }, [refreshDeployment]);

  useEffect(() => {
    void refreshPreferences();
    return () => {
      preferenceRequest.current += 1;
    };
  }, [refreshPreferences]);

  useEffect(() => {
    let active = true;
    void client
      .getSdkSchema()
      .then((schema) => {
        if (active) {
          setSdkState({ status: "ready", schema });
        }
      })
      .catch((error: unknown) => {
        if (active) {
          setSdkState({
            status: "error",
            error: parseModStudioCommandError(error),
          });
        }
      });
    return () => {
      active = false;
    };
  }, [client]);

  useEffect(() => {
    const unsubscribe = client.subscribeRuntime(
      (event) => {
        setRuntimeState((current) => acceptRuntimeEvent(current, event));
      },
      (error) => {
        setRuntimeState((current) => ({
          ...current,
          connection: "probeFailed",
          bootstrapErrorCode: null,
          error,
        }));
      },
    );
    return () => {
      void unsubscribe().catch((error: unknown) => {
        console.error("Mod runtime subscription cleanup failed", error);
      });
    };
  }, [client]);

  const loadingDocumentId =
    state.status === "ready" && state.document.status === "loading"
      ? state.selectedId
      : null;

  useEffect(() => {
    if (loadingDocumentId === null) {
      return;
    }
    const id = loadingDocumentId;
    let active = true;
    void client
      .getDocument(id)
      .then((document) => {
        if (active) {
          setState((current) => acceptDocument(current, document));
        }
      })
      .catch((error: unknown) => {
        if (active) {
          setState((current) =>
            rejectDocument(current, id, parseModStudioCommandError(error)),
          );
        }
      });
    return () => {
      active = false;
    };
  }, [client, loadingDocumentId]);

  const loadedDocument =
    state.status === "ready" && state.document.status === "ready"
      ? state.document.document
      : null;

  useEffect(() => {
    if (loadedDocument === null) {
      return;
    }
    setBuffers((current) => ({
      ...current,
      [loadedDocument.id]: mergeLoadedSource(
        current[loadedDocument.id],
        loadedDocument.source,
      ),
    }));
  }, [loadedDocument]);

  const editSource = useCallback(
    (id: string, savedSource: string, source: string) => {
      setBuffers((current) => ({
        ...current,
        [id]: editSourceBuffer(
          current[id] ?? { source: savedSource, savedSource },
          source,
        ),
      }));
      setSaveStates((current) => ({
        ...current,
        [id]: { status: "idle" },
      }));
    },
    [],
  );

  const revertSource = useCallback(
    (id: string, buffer: ModStudioSourceBuffer) => {
      setBuffers((current) => ({
        ...current,
        [id]: revertSourceBuffer(current[id] ?? buffer),
      }));
      setSaveStates((current) => ({
        ...current,
        [id]: { status: "idle" },
      }));
    },
    [],
  );

  const saveSource = useCallback(
    async (id: string, buffer: ModStudioSourceBuffer) => {
      if (savingIds.current.has(id)) {
        return;
      }
      savingIds.current.add(id);
      const submittedSource = buffer.source;
      setSaveStates((current) => ({
        ...current,
        [id]: { status: "saving" },
      }));
      try {
        const document = await client.saveDocument(id, submittedSource);
        setBuffers((current) => ({
          ...current,
          [id]: acceptSavedSource(
            current[id] ?? buffer,
            submittedSource,
            document.source,
          ),
        }));
        setState((current) => acceptSavedDocument(current, document));
        setSaveStates((current) => ({
          ...current,
          [id]: { status: "saved" },
        }));
      } catch (error) {
        setSaveStates((current) => ({
          ...current,
          [id]: {
            status: "error",
            error: parseModStudioCommandError(error),
          },
        }));
      } finally {
        savingIds.current.delete(id);
      }
    },
    [client],
  );

  const setDocumentEnabled = useCallback(
    async (id: string, enabled: boolean) => {
      const buffer = buffers[id];
      if (
        enablingIds.current.has(id) ||
        (buffer !== undefined && isSourceBufferDirty(buffer))
      ) {
        return;
      }
      enablingIds.current.add(id);
      setEnableStates((current) => ({
        ...current,
        [id]: { status: "saving" },
      }));
      try {
        const workspace = await client.setEnabled(id, enabled);
        setState((current) => acceptEnabledWorkspace(current, workspace));
        setEnableStates((current) => ({
          ...current,
          [id]: { status: "idle" },
        }));
      } catch (error) {
        setEnableStates((current) => ({
          ...current,
          [id]: {
            status: "error",
            error: parseModStudioCommandError(error),
          },
        }));
      } finally {
        enablingIds.current.delete(id);
      }
    },
    [buffers, client],
  );

  const createDocument = useCallback(
    async (id: string) => {
      setCreateState({ status: "working" });
      try {
        const document = await client.createDocument(id);
        const workspace = await client.getWorkspace();
        selectedId.current = id;
        setState(acceptDocument(acceptWorkspace(workspace, id), document));
        setBuffers((current) => ({
          ...current,
          [id]: { source: document.source, savedSource: document.source },
        }));
        setCreateState({ status: "done" });
      } catch (error) {
        setCreateState({
          status: "error",
          error: parseModStudioCommandError(error),
        });
      }
    },
    [client],
  );

  const deleteDocument = useCallback(
    async (id: string) => {
      const buffer = buffers[id];
      if (
        deletingIds.current.has(id) ||
        savingIds.current.has(id) ||
        enablingIds.current.has(id) ||
        (buffer !== undefined && isSourceBufferDirty(buffer))
      ) {
        return;
      }
      deletingIds.current.add(id);
      setDeleteStates((current) => ({
        ...current,
        [id]: { status: "deleting" },
      }));
      try {
        const workspace = await client.deleteDocument(id);
        setState((current) => {
          const next = acceptEnabledWorkspace(current, workspace);
          selectedId.current = selectedIdOf(next);
          return next;
        });
        setBuffers((current) => {
          const next = { ...current };
          delete next[id];
          return next;
        });
        setSaveStates((current) => {
          const next = { ...current };
          delete next[id];
          return next;
        });
        setEnableStates((current) => {
          const next = { ...current };
          delete next[id];
          return next;
        });
        setDeleteStates((current) => {
          const next = { ...current };
          delete next[id];
          return next;
        });
      } catch (error) {
        setDeleteStates((current) => ({
          ...current,
          [id]: {
            status: "error",
            error: parseModStudioCommandError(error),
          },
        }));
      } finally {
        deletingIds.current.delete(id);
      }
    },
    [buffers, client],
  );

  const openFolder = useCallback(async () => {
    setFolderState({ status: "working" });
    try {
      await client.openFolder();
      setFolderState({ status: "done" });
    } catch (error) {
      setFolderState({
        status: "error",
        error: parseModStudioCommandError(error),
      });
    }
  }, [client]);

  const openLoaderDirectory = useCallback(async () => {
    setLoaderDirectoryState({ status: "working" });
    try {
      await client.openLoaderDirectory();
      setLoaderDirectoryState({ status: "done" });
    } catch (error) {
      setLoaderDirectoryState({
        status: "error",
        error: parseModStudioCommandError(error),
      });
    }
  }, [client]);

  const selectLoadingMethod = useCallback(
    async (method: ModLoadingMethod) => {
      if (method === loadingMethod) return;
      const request = ++preferenceRequest.current;
      const previous = loadingMethod;
      const riskAcknowledged =
        preferenceState.status === "ready"
          ? preferenceState.riskAcknowledged
          : false;
      setLoadingMethod(method);
      setPreferenceState({
        status: "ready",
        riskAcknowledged,
        operation: "updating",
      });
      try {
        const preference = await client.setLoadingMethod(method);
        if (request !== preferenceRequest.current) return;
        setLoadingMethod(preference.method);
        setPreferenceState({
          status: "ready",
          riskAcknowledged: preference.riskAcknowledged,
          operation: "idle",
        });
      } catch (error) {
        if (request !== preferenceRequest.current) return;
        setLoadingMethod(previous);
        setPreferenceState({
          status: "error",
          error: parseModStudioCommandError(error),
        });
      }
    },
    [client, loadingMethod, preferenceState],
  );

  const acknowledgeRisk = useCallback(async (): Promise<boolean> => {
    const request = ++preferenceRequest.current;
    setPreferenceState((current) =>
      current.status === "ready"
        ? { ...current, operation: "updating" }
        : current,
    );
    try {
      const preference = await client.acknowledgeRisk();
      if (request !== preferenceRequest.current) return false;
      setLoadingMethod(preference.method);
      setPreferenceState({
        status: "ready",
        riskAcknowledged: preference.riskAcknowledged,
        operation: "idle",
      });
      return preference.riskAcknowledged;
    } catch (error) {
      if (request !== preferenceRequest.current) return false;
      setPreferenceState({
        status: "error",
        error: parseModStudioCommandError(error),
      });
      return false;
    }
  }, [client]);

  const chooseGameDirectory = useCallback(async () => {
    deploymentRequest.current += 1;
    initialRegionResolved.current = true;
    setDeploymentState((current) =>
      current.status === "ready"
        ? { ...current, operation: "choosing" }
        : current,
    );
    try {
      const selection = await client.chooseGameDirectory(selectedRegion);
      if (!selection.selected || selection.path === null) {
        setDeploymentState((current) =>
          current.status === "ready"
            ? { ...current, operation: "idle" }
            : current,
        );
        return;
      }
      setManualDirectories((current) => ({
        ...current,
        [selectedRegion]: selection.path ?? undefined,
      }));
      setDeploymentState({
        status: "ready",
        snapshot: selection.deployment,
        operation: "idle",
      });
    } catch (error) {
      setDeploymentState({
        status: "error",
        error: parseModStudioCommandError(error),
      });
    }
  }, [client, selectedRegion]);

  const useAutomaticGameDirectory = useCallback(async () => {
    deploymentRequest.current += 1;
    setDeploymentState((current) =>
      current.status === "ready"
        ? { ...current, operation: "updating" }
        : current,
    );
    try {
      await client.setGameDirectory(selectedRegion, null);
      await refreshDeployment();
    } catch (error) {
      setDeploymentState({
        status: "error",
        error: parseModStudioCommandError(error),
      });
    }
  }, [client, refreshDeployment, selectedRegion]);

  const setProxyEnabled = useCallback(
    async (enabled: boolean) => {
      const directory = manualDirectories[selectedRegion] ?? null;
      setDeploymentState((current) =>
        current.status === "ready"
          ? { ...current, operation: "updating" }
          : current,
      );
      try {
        if (
          enabled &&
          loaderState.status === "ready" &&
          loaderState.snapshot.phase === "running"
        ) {
          const loaderSnapshot = await client.setLoaderRunning(false);
          setLoaderState({
            status: "ready",
            snapshot: loaderSnapshot,
            operation: "idle",
          });
        }
        const snapshot = await client.setLoaderEnabled(
          selectedRegion,
          enabled,
          directory,
        );
        setDeploymentState({ status: "ready", snapshot, operation: "idle" });
        await refresh();
      } catch (error) {
        setDeploymentState({
          status: "error",
          error: parseModStudioCommandError(error),
        });
      }
    },
    [client, loaderState, manualDirectories, refresh, selectedRegion],
  );

  const setLoaderRunning = useCallback(
    async (running: boolean) => {
      loaderRequest.current += 1;
      setLoaderState((current) =>
        current.status === "ready"
          ? { ...current, operation: "updating" }
          : current,
      );
      try {
        if (running && deploymentState.status === "ready") {
          const selectedGame = deploymentState.snapshot.games.find(
            (game) => game.region === selectedRegion,
          );
          if (selectedGame?.installed) {
            const directory = manualDirectories[selectedRegion] ?? null;
            const deploymentSnapshot = await client.setLoaderEnabled(
              selectedRegion,
              false,
              directory,
            );
            setDeploymentState({
              status: "ready",
              snapshot: deploymentSnapshot,
              operation: "idle",
            });
          }
        }
        const snapshot = await client.setLoaderRunning(running);
        setLoaderState({ status: "ready", snapshot, operation: "idle" });
        if (running) {
          await refresh();
        }
      } catch (error) {
        setLoaderState({
          status: "error",
          error: parseModStudioCommandError(error),
        });
      }
    },
    [client, deploymentState, manualDirectories, refresh, selectedRegion],
  );

  const retryLoader = useCallback(async () => {
    setLoaderState((current) =>
      current.status === "ready" ? { ...current, operation: "idle" } : current,
    );
    await refreshLoader();
  }, [refreshLoader]);

  const selectedBuffer =
    loadedDocument === null
      ? null
      : (buffers[loadedDocument.id] ?? {
          source: loadedDocument.source,
          savedSource: loadedDocument.source,
        });
  const selectedSaveState =
    state.status === "ready"
      ? (saveStates[state.selectedId] ?? { status: "idle" as const })
      : ({ status: "idle" } as const);
  const dirtyDocumentIds = new Set(
    Object.entries(buffers)
      .filter(([, buffer]) => isSourceBufferDirty(buffer))
      .map(([id]) => id),
  );

  return {
    state,
    refresh: refreshAll,
    chooseDocument,
    selectedBuffer,
    selectedSaveState,
    selectedBufferDirty:
      selectedBuffer !== null && isSourceBufferDirty(selectedBuffer),
    dirtyDocumentIds,
    editSource,
    revertSource,
    saveSource,
    enableStates,
    setDocumentEnabled,
    createDocument,
    deleteDocument,
    deleteStates,
    createState,
    openFolder,
    folderState,
    loadingMethod,
    preferenceState,
    setLoadingMethod: selectLoadingMethod,
    acknowledgeRisk,
    selectedRegion,
    setSelectedRegion: setSelectedRegionState,
    manualGameDirectory: manualDirectories[selectedRegion] ?? null,
    deploymentState,
    refreshDeployment,
    chooseGameDirectory,
    useAutomaticGameDirectory,
    setProxyEnabled,
    loaderState,
    retryLoader,
    setLoaderRunning,
    openLoaderDirectory,
    loaderDirectoryState,
    runtimeState,
    sdkState,
  };
}
