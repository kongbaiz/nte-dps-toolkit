import { useCallback, useEffect, useRef, useState } from "react";

import {
  modStudioClient,
  type ModStudioClient,
} from "@/lib/tauri/mod-studio-client";
import { parseModStudioCommandError } from "@/lib/tauri/mod-studio-contract";

import {
  acceptDocument,
  acceptWorkspace,
  rejectDocument,
  selectDocument,
  selectedIdOf,
  type ModStudioPageState,
} from "./mod-studio-view-model";

export function useModStudio(client: ModStudioClient = modStudioClient) {
  const [state, setState] = useState<ModStudioPageState>({
    status: "loading",
  });
  const workspaceRequest = useRef(0);
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

  return {
    state,
    refresh,
    chooseDocument,
  };
}
