import { describe, expect, it } from "vitest";

import {
  ModStudioContractError,
  compareModStudioSequence,
  parseModStudioCommandError,
  parseModStudioDeployment,
  parseModStudioDirectorySelection,
  parseModStudioDocument,
  parseModStudioRuntimeEvent,
  parseModStudioSdkSchema,
  parseModStudioSubscriptionReceipt,
  parseModStudioWorkspace,
} from "./mod-studio-contract";

describe("Mod Studio contract", () => {
  it("parses a bounded workspace index without source bodies", () => {
    expect(
      parseModStudioWorkspace({
        contractVersion: 5,
        generation: "7",
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
      contractVersion: 5,
      generation: "7",
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
        contractVersion: 6,
        generation: "0",
        workspaceLabel: "plugins/nte-mods",
        documents: [],
      }),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModStudioWorkspace({
        contractVersion: 5,
        generation: "0",
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
        contractVersion: 5,
        generation: "0",
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
        contractVersion: 5,
        id: "combat-clock",
        enabled: true,
        source: "NTE_SCRIPT(5);",
      }),
    ).toEqual({
      contractVersion: 5,
      id: "combat-clock",
      enabled: true,
      source: "NTE_SCRIPT(5);",
    });
  });

  it("parses bounded deployment state and a validated manual path selection", () => {
    const deployment = {
      contractVersion: 5,
      installations: 1,
      installed: 1,
      current: 1,
      sourceAvailable: true,
      games: [{ region: "global", installed: true, current: true }],
    };

    expect(parseModStudioDeployment(deployment)).toEqual(deployment);
    expect(
      parseModStudioDirectorySelection({
        selected: true,
        path: "D:\\Game",
        deployment,
      }),
    ).toMatchObject({ selected: true, path: "D:\\Game" });
    expect(() =>
      parseModStudioDirectorySelection({
        selected: false,
        path: "D:\\Game",
        deployment,
      }),
    ).toThrow(ModStudioContractError);
    expect(() =>
      parseModStudioDeployment({
        ...deployment,
        installed: 2,
      }),
    ).toThrow(ModStudioContractError);
  });

  it("parses the bounded versioned Mod SDK schema", () => {
    expect(
      parseModStudioSdkSchema({
        contractVersion: 5,
        schemaVersion: 1,
        symbols: [
          {
            label: "nte::time::now_ms()",
            insertText: "nte::time::now_ms()",
            kind: "function",
            returnType: "std::uint64_t",
            documentationKey: "Monotonic process time in milliseconds.",
          },
        ],
      }),
    ).toMatchObject({
      schemaVersion: 1,
      symbols: [{ kind: "function", returnType: "std::uint64_t" }],
    });
    expect(() =>
      parseModStudioSdkSchema({
        contractVersion: 5,
        schemaVersion: 2,
        symbols: [],
      }),
    ).toThrow(ModStudioContractError);
  });

  it("parses optional source diagnostics and rejects invalid diagnostic lines", () => {
    expect(
      parseModStudioCommandError({
        code: "mod_source_invalid_line",
        messageKey: "The NTE C++ compiler rejected line {0}.",
        messageArguments: ["23"],
        diagnosticLine: 23,
      }),
    ).toEqual({
      code: "mod_source_invalid_line",
      messageKey: "The NTE C++ compiler rejected line {0}.",
      messageArguments: ["23"],
      diagnosticLine: 23,
    });
    expect(() =>
      parseModStudioCommandError({
        code: "mod_source_invalid_line",
        messageKey: "The NTE C++ compiler rejected line {0}.",
        messageArguments: ["0"],
        diagnosticLine: 0,
      }),
    ).toThrow(ModStudioContractError);
    expect(
      parseModStudioCommandError({
        code: "mod_source_write_failed",
        messageKey: "Failed to save the Mod source.",
        messageArguments: [],
        diagnosticLine: null,
      }).diagnosticLine,
    ).toBeNull();
  });

  it("parses ordered bounded runtime batches with u64 string fields", () => {
    expect(
      parseModStudioRuntimeEvent({
        event: "batch",
        payload: {
          contractVersion: 5,
          generation: "2",
          entries: [
            {
              kind: "log",
              sequence: "9",
              nativeSequence: "4",
              timestamp100ns: "133000000000000000",
              modId: "runtime",
              level: "warning",
              message: "Compilation failed; previous version kept.",
              messageKey: "Compilation failed; previous version kept.",
              messageArguments: [],
            },
            {
              kind: "event",
              sequence: "10",
              nativeSequence: "7",
              timestamp100ns: "133000000000000010",
              modId: "telemetry",
              name: "post.sample",
              values: ["1", "18446744073709551615"],
            },
          ],
        },
      }),
    ).toMatchObject({
      event: "batch",
      payload: {
        generation: "2",
        entries: [{ sequence: "9" }, { sequence: "10" }],
      },
    });
    expect(() =>
      parseModStudioRuntimeEvent({
        event: "batch",
        payload: {
          contractVersion: 5,
          generation: "2",
          entries: [
            {
              kind: "log",
              sequence: "10",
              nativeSequence: "10",
              timestamp100ns: "1",
              modId: "runtime",
              level: "info",
              message: "Hot reload applied.",
              messageKey: "Hot reload applied.",
              messageArguments: [],
            },
            {
              kind: "log",
              sequence: "9",
              nativeSequence: "11",
              timestamp100ns: "2",
              modId: "runtime",
              level: "info",
              message: "Hot reload applied.",
              messageKey: "Hot reload applied.",
              messageArguments: [],
            },
          ],
        },
      }),
    ).toThrow(ModStudioContractError);
    expect(compareModStudioSequence("9", "10")).toBeLessThan(0);
  });

  it("parses runtime connection receipts", () => {
    expect(
      parseModStudioSubscriptionReceipt({
        subscriptionId: "runtime-01",
        streamIntervalMs: 250,
      }),
    ).toEqual({
      subscriptionId: "runtime-01",
      streamIntervalMs: 250,
    });
    expect(
      parseModStudioRuntimeEvent({
        event: "connection",
        payload: {
          contractVersion: 5,
          generation: "3",
          connected: false,
        },
      }),
    ).toMatchObject({
      event: "connection",
      payload: { generation: "3", connected: false },
    });
  });
});
