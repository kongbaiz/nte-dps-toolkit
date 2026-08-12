import type {
  ModStudioCommandError,
  ModStudioDocumentSnapshot,
  ModStudioRuntimeEntry,
  ModStudioRuntimeEvent,
  ModStudioWorkspaceSnapshot,
} from "@/lib/tauri/mod-studio-contract";
import {
  compareModStudioSequence,
  MOD_STUDIO_MAX_RUNTIME_ENTRIES,
} from "@/lib/tauri/mod-studio-contract";

export type ModStudioDocumentState =
  | { status: "loading" }
  | { status: "error"; error: ModStudioCommandError }
  | { status: "ready"; document: ModStudioDocumentSnapshot };

export type ModStudioPageState =
  | { status: "loading" }
  | { status: "error"; error: ModStudioCommandError }
  | { status: "empty"; workspace: ModStudioWorkspaceSnapshot }
  | {
      status: "ready";
      workspace: ModStudioWorkspaceSnapshot;
      selectedId: string;
      document: ModStudioDocumentState;
    };

export interface ModStudioSourceBuffer {
  source: string;
  savedSource: string;
}

export interface ModStudioRuntimeState {
  generation: string | null;
  connection:
    "connecting" | "connected" | "loaderPresent" | "waiting" | "probeFailed";
  entries: ModStudioRuntimeEntry[];
  error: ModStudioCommandError | null;
}

export const INITIAL_MOD_STUDIO_RUNTIME_STATE: ModStudioRuntimeState = {
  generation: null,
  connection: "connecting",
  entries: [],
  error: null,
};

export function acceptWorkspace(
  workspace: ModStudioWorkspaceSnapshot,
  preferredId: string | null,
): ModStudioPageState {
  if (workspace.documents.length === 0) {
    return { status: "empty", workspace };
  }
  const selectedId =
    preferredId !== null &&
    workspace.documents.some((document) => document.id === preferredId)
      ? preferredId
      : workspace.documents[0].id;
  return {
    status: "ready",
    workspace,
    selectedId,
    document: { status: "loading" },
  };
}

export function selectDocument(
  state: ModStudioPageState,
  id: string,
): ModStudioPageState {
  if (
    state.status !== "ready" ||
    state.selectedId === id ||
    !state.workspace.documents.some((document) => document.id === id)
  ) {
    return state;
  }
  return {
    ...state,
    selectedId: id,
    document: { status: "loading" },
  };
}

export function acceptDocument(
  state: ModStudioPageState,
  document: ModStudioDocumentSnapshot,
): ModStudioPageState {
  if (state.status !== "ready" || state.selectedId !== document.id) {
    return state;
  }
  return {
    ...state,
    document: { status: "ready", document },
  };
}

export function acceptSavedDocument(
  state: ModStudioPageState,
  document: ModStudioDocumentSnapshot,
): ModStudioPageState {
  if (state.status !== "ready" || state.selectedId !== document.id) {
    return state;
  }
  const sourceBytes = new TextEncoder().encode(document.source).length;
  const splitLineCount = document.source.split("\n").length;
  const lineCount =
    document.source.length === 0
      ? 0
      : splitLineCount - Number(document.source.endsWith("\n"));
  return {
    ...state,
    workspace: {
      ...state.workspace,
      documents: state.workspace.documents.map((summary) =>
        summary.id === document.id
          ? { ...summary, sourceBytes, lineCount }
          : summary,
      ),
    },
    document: { status: "ready", document },
  };
}

export function acceptEnabledWorkspace(
  state: ModStudioPageState,
  workspace: ModStudioWorkspaceSnapshot,
): ModStudioPageState {
  if (
    (state.status === "ready" || state.status === "empty") &&
    compareModStudioSequence(workspace.generation, state.workspace.generation) <
      0
  ) {
    return state;
  }
  if (state.status !== "ready") {
    return acceptWorkspace(workspace, null);
  }
  const selected = workspace.documents.find(
    (document) => document.id === state.selectedId,
  );
  if (selected === undefined) {
    return acceptWorkspace(workspace, state.selectedId);
  }
  return {
    ...state,
    workspace,
    document:
      state.document.status === "ready"
        ? {
            status: "ready",
            document: {
              ...state.document.document,
              enabled: selected.enabled,
            },
          }
        : state.document,
  };
}

export function rejectDocument(
  state: ModStudioPageState,
  id: string,
  error: ModStudioCommandError,
): ModStudioPageState {
  if (state.status !== "ready" || state.selectedId !== id) {
    return state;
  }
  return {
    ...state,
    document: { status: "error", error },
  };
}

export function selectedIdOf(state: ModStudioPageState): string | null {
  return state.status === "ready" ? state.selectedId : null;
}

export function mergeLoadedSource(
  buffer: ModStudioSourceBuffer | undefined,
  source: string,
): ModStudioSourceBuffer {
  if (buffer === undefined || buffer.source === buffer.savedSource) {
    return { source, savedSource: source };
  }
  return buffer;
}

export function editSourceBuffer(
  buffer: ModStudioSourceBuffer,
  source: string,
): ModStudioSourceBuffer {
  return { ...buffer, source };
}

export function revertSourceBuffer(
  buffer: ModStudioSourceBuffer,
): ModStudioSourceBuffer {
  return { ...buffer, source: buffer.savedSource };
}

export function acceptSavedSource(
  buffer: ModStudioSourceBuffer,
  submittedSource: string,
  savedSource: string,
): ModStudioSourceBuffer {
  return {
    source: buffer.source === submittedSource ? savedSource : buffer.source,
    savedSource,
  };
}

export function isSourceBufferDirty(buffer: ModStudioSourceBuffer): boolean {
  return buffer.source !== buffer.savedSource;
}

export function acceptRuntimeEvent(
  state: ModStudioRuntimeState,
  event: ModStudioRuntimeEvent,
): ModStudioRuntimeState {
  if (event.event === "connection") {
    const reset = state.generation !== event.payload.generation;
    return {
      generation: event.payload.generation,
      connection: event.payload.status,
      entries: reset ? [] : state.entries,
      error: null,
    };
  }

  const existing =
    state.generation === event.payload.generation ? state.entries : [];
  const bySequence = new Map(
    existing.map((entry) => [entry.sequence, entry] as const),
  );
  for (const entry of event.payload.entries) {
    bySequence.set(entry.sequence, entry);
  }
  const entries = [...bySequence.values()]
    .sort((left, right) =>
      compareModStudioSequence(left.sequence, right.sequence),
    )
    .slice(-MOD_STUDIO_MAX_RUNTIME_ENTRIES);
  return {
    generation: event.payload.generation,
    connection: "connected",
    entries,
    error: null,
  };
}
