import { useCallback, useEffect, useRef, useState } from "react";

import {
  modStudioClient,
  type ModStudioClient,
} from "@/lib/tauri/mod-studio-client";
import {
  parseModStudioCommandError,
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

export function useModStudio(client: ModStudioClient = modStudioClient) {
  const [state, setState] = useState<ModStudioPageState>({
    status: "loading",
  });
  const workspaceRequest = useRef(0);
  const savingIds = useRef(new Set<string>());
  const enablingIds = useRef(new Set<string>());
  const [buffers, setBuffers] = useState<Record<string, ModStudioSourceBuffer>>(
    {},
  );
  const [saveStates, setSaveStates] = useState<
    Record<string, ModStudioSaveState>
  >({});
  const [enableStates, setEnableStates] = useState<
    Record<string, ModStudioEnableState>
  >({});
  const [runtimeState, setRuntimeState] = useState(
    INITIAL_MOD_STUDIO_RUNTIME_STATE,
  );
  const [sdkState, setSdkState] = useState<ModStudioSdkState>({
    status: "loading",
  });
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
          connection: "disconnected",
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
    refresh,
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
    runtimeState,
    sdkState,
  };
}
