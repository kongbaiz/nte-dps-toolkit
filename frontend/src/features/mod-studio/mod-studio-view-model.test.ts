import { describe, expect, it } from "vitest";

import type {
  ModStudioCommandError,
  ModStudioWorkspaceSnapshot,
} from "@/lib/tauri/mod-studio-contract";

import {
  acceptDocument,
  acceptWorkspace,
  rejectDocument,
  selectDocument,
} from "./mod-studio-view-model";

const workspace: ModStudioWorkspaceSnapshot = {
  contractVersion: 1,
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
      contractVersion: 1,
      id: "combat-clock",
      enabled: true,
      source: "old",
    });

    expect(stale).toBe(telemetry);
    expect(
      acceptDocument(telemetry, {
        contractVersion: 1,
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
});
