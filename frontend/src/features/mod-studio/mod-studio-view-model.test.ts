import { describe, expect, it } from "vitest";

import type {
  ModStudioCommandError,
  ModStudioWorkspaceSnapshot,
} from "@/lib/tauri/mod-studio-contract";

import {
  acceptDocument,
  acceptEnabledWorkspace,
  acceptRuntimeEvent,
  acceptSavedDocument,
  acceptSavedSource,
  acceptWorkspace,
  editSourceBuffer,
  INITIAL_MOD_STUDIO_RUNTIME_STATE,
  isSourceBufferDirty,
  mergeLoadedSource,
  rejectDocument,
  revertSourceBuffer,
  selectDocument,
} from "./mod-studio-view-model";

const workspace: ModStudioWorkspaceSnapshot = {
  contractVersion: 13,
  generation: "0",
  workspaceLabel: "plugins/nte-mods",
  documents: [
    {
      id: "combat-clock",
      enabled: true,
      sourceBytes: 100,
      lineCount: 10,
    },
    {
      id: "telemetry",
      enabled: false,
      sourceBytes: 200,
      lineCount: 20,
    },
  ],
};
const error: ModStudioCommandError = {
  code: "mod_document_not_found",
  messageKey: "The selected Mod document no longer exists.",
  messageArguments: [],
  diagnosticLine: null,
};

describe("Mod Studio view model", () => {
  it("projects empty workspaces and preserves a valid selection on refresh", () => {
    expect(acceptWorkspace({ ...workspace, documents: [] }, null).status).toBe(
      "empty",
    );
    expect(acceptWorkspace(workspace, "telemetry")).toMatchObject({
      status: "ready",
      selectedId: "telemetry",
      document: { status: "loading" },
    });
    expect(acceptWorkspace(workspace, "removed")).toMatchObject({
      selectedId: "combat-clock",
    });
  });

  it("loads only known selections", () => {
    const ready = acceptWorkspace(workspace, null);
    const selected = selectDocument(ready, "telemetry");

    expect(selected).toMatchObject({
      selectedId: "telemetry",
      document: { status: "loading" },
    });
    expect(selectDocument(selected, "../telemetry")).toBe(selected);
  });

  it("ignores late detail responses from the previous selection", () => {
    const telemetry = selectDocument(
      acceptWorkspace(workspace, null),
      "telemetry",
    );
    const stale = acceptDocument(telemetry, {
      contractVersion: 13,
      id: "combat-clock",
      enabled: true,
      source: "old",
    });

    expect(stale).toBe(telemetry);
    expect(
      acceptDocument(telemetry, {
        contractVersion: 13,
        id: "telemetry",
        enabled: false,
        source: "current",
      }),
    ).toMatchObject({
      document: {
        status: "ready",
        document: { id: "telemetry", source: "current" },
      },
    });
  });

  it("keeps document errors scoped to the selected source", () => {
    const ready = acceptWorkspace(workspace, null);

    expect(rejectDocument(ready, "combat-clock", error)).toMatchObject({
      status: "ready",
      document: { status: "error", error },
    });
    expect(rejectDocument(ready, "telemetry", error)).toBe(ready);
  });

  it("preserves dirty buffers across document loads and selection changes", () => {
    const clean = mergeLoadedSource(undefined, "original");
    const dirty = editSourceBuffer(clean, "edited");

    expect(mergeLoadedSource(dirty, "external")).toBe(dirty);
    expect(isSourceBufferDirty(dirty)).toBe(true);
    expect(revertSourceBuffer(dirty)).toEqual({
      source: "original",
      savedSource: "original",
    });
  });

  it("accepts a save without discarding edits made while it was in flight", () => {
    const submitted = {
      source: "submitted",
      savedSource: "original",
    };
    expect(acceptSavedSource(submitted, "submitted", "submitted")).toEqual({
      source: "submitted",
      savedSource: "submitted",
    });
    expect(
      acceptSavedSource(
        { source: "newer edit", savedSource: "original" },
        "submitted",
        "submitted",
      ),
    ).toEqual({
      source: "newer edit",
      savedSource: "submitted",
    });
  });

  it("updates the selected document summary after a save", () => {
    const ready = acceptWorkspace(workspace, null);
    const saved = acceptSavedDocument(ready, {
      contractVersion: 13,
      id: "combat-clock",
      enabled: true,
      source: "one\ntwo\nthree",
    });

    expect(saved).toMatchObject({
      document: {
        status: "ready",
        document: { source: "one\ntwo\nthree" },
      },
    });
    expect(
      saved.status === "ready"
        ? saved.workspace.documents.find(
            (document) => document.id === "combat-clock",
          )
        : undefined,
    ).toMatchObject({ sourceBytes: 13, lineCount: 3 });
  });

  it("updates enabled state without discarding the selected document", () => {
    const ready = acceptDocument(acceptWorkspace(workspace, null), {
      contractVersion: 13,
      id: "combat-clock",
      enabled: true,
      source: "saved source",
    });
    const updated = acceptEnabledWorkspace(ready, {
      ...workspace,
      generation: "2",
      documents: workspace.documents.map((document) =>
        document.id === "combat-clock"
          ? { ...document, enabled: false }
          : document,
      ),
    });

    expect(updated).toMatchObject({
      selectedId: "combat-clock",
      document: {
        status: "ready",
        document: { enabled: false, source: "saved source" },
      },
    });

    const stale = acceptEnabledWorkspace(updated, {
      ...workspace,
      generation: "1",
    });
    expect(stale).toBe(updated);
  });

  it("selects the next document after the selected Mod is deleted", () => {
    const ready = acceptWorkspace(workspace, "combat-clock");
    const afterDelete = acceptEnabledWorkspace(ready, {
      contractVersion: 13,
      generation: "9",
      workspaceLabel: "plugins/nte-mods",
      documents: [workspace.documents[1]],
    });

    expect(afterDelete).toMatchObject({
      status: "ready",
      selectedId: "telemetry",
      document: { status: "loading" },
    });
  });

  it("deduplicates bounded runtime batches and resets on a new generation", () => {
    const connected = acceptRuntimeEvent(INITIAL_MOD_STUDIO_RUNTIME_STATE, {
      event: "connection",
      payload: {
        contractVersion: 13,
        generation: "1",
        status: "connected",
        bootstrapErrorCode: null,
        probeErrorCode: null,
        probeOsErrorCode: null,
      },
    });
    const message = {
      kind: "log" as const,
      sequence: "9",
      nativeSequence: "4",
      timestamp100ns: "133000000000000000",
      modId: "runtime",
      level: "info" as const,
      message: "Hot reload applied.",
      messageKey: "Hot reload applied.",
      messageArguments: [],
    };
    const received = acceptRuntimeEvent(connected, {
      event: "batch",
      payload: {
        contractVersion: 13,
        generation: "1",
        entries: [message],
      },
    });
    const repeated = acceptRuntimeEvent(received, {
      event: "batch",
      payload: {
        contractVersion: 13,
        generation: "1",
        entries: [message],
      },
    });
    const reset = acceptRuntimeEvent(repeated, {
      event: "connection",
      payload: {
        contractVersion: 13,
        generation: "2",
        status: "connected",
        bootstrapErrorCode: null,
        probeErrorCode: null,
        probeOsErrorCode: null,
      },
    });

    expect(received.entries).toHaveLength(1);
    expect(repeated.entries).toHaveLength(1);
    expect(reset).toMatchObject({
      generation: "2",
      connection: "connected",
      entries: [],
    });
  });

  it("keeps a loaded loader distinct from a connected game hook", () => {
    const loaded = acceptRuntimeEvent(INITIAL_MOD_STUDIO_RUNTIME_STATE, {
      event: "connection",
      payload: {
        contractVersion: 13,
        generation: "1",
        status: "loaderPresent",
        bootstrapErrorCode: null,
        probeErrorCode: null,
        probeOsErrorCode: null,
      },
    });

    expect(loaded.connection).toBe("loaderPresent");
  });

  it("preserves precise runtime probe diagnostics", () => {
    const failed = acceptRuntimeEvent(INITIAL_MOD_STUDIO_RUNTIME_STATE, {
      event: "connection",
      payload: {
        contractVersion: 13,
        generation: "1",
        status: "probeFailed",
        bootstrapErrorCode: null,
        probeErrorCode: "IPC_PIPE_ACCESS_DENIED",
        probeOsErrorCode: 5,
      },
    });

    expect(failed).toMatchObject({
      connection: "probeFailed",
      probeErrorCode: "IPC_PIPE_ACCESS_DENIED",
      probeOsErrorCode: 5,
    });
  });
});
