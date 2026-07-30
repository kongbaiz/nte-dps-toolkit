import { describe, expect, it } from "vitest";

import {
  ModStudioContractError,
  parseModStudioDocument,
  parseModStudioWorkspace,
} from "./mod-studio-contract";

describe("Mod Studio contract", () => {
  it("parses a bounded workspace index without source bodies", () => {
    expect(
      parseModStudioWorkspace({
        contractVersion: 1,
        workspaceLabel: "plugins/nte-mods",
        documents: [
          {
            id: "combat-clock",
            enabled: true,
            sourceBytes: 1200,
            lineCount: 42,
          },
        ],
      }),
    ).toEqual({
      contractVersion: 1,
      workspaceLabel: "plugins/nte-mods",
      documents: [
        {
          id: "combat-clock",
          enabled: true,
          sourceBytes: 1200,
          lineCount: 42,
        },
      ],
    });
  });

  it("rejects unknown versions, invalid IDs, and duplicate documents", () => {
    expect(() =>
      parseModStudioWorkspace({
        contractVersion: 2,
        workspaceLabel: "plugins/nte-mods",
        documents: [],
      }),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModStudioWorkspace({
        contractVersion: 1,
        workspaceLabel: "plugins/nte-mods",
        documents: [
          {
            id: "../combat-clock",
            enabled: true,
            sourceBytes: 1,
            lineCount: 1,
          },
        ],
      }),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModStudioWorkspace({
        contractVersion: 1,
        workspaceLabel: "plugins/nte-mods",
        documents: [
          {
            id: "combat-clock",
            enabled: true,
            sourceBytes: 1,
            lineCount: 1,
          },
          {
            id: "combat-clock",
            enabled: false,
            sourceBytes: 2,
            lineCount: 2,
          },
        ],
      }),
    ).toThrow(ModStudioContractError);
  });

  it("parses a requested document body", () => {
    expect(
      parseModStudioDocument({
        contractVersion: 1,
        id: "combat-clock",
        enabled: true,
        source: "NTE_SCRIPT(5);",
      }),
    ).toEqual({
      contractVersion: 1,
      id: "combat-clock",
      enabled: true,
      source: "NTE_SCRIPT(5);",
    });
  });
});
