import type {
  ModStudioCommandError,
  ModStudioDocumentSnapshot,
  ModStudioWorkspaceSnapshot,
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
