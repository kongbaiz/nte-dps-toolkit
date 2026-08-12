import { describe, expect, it } from "vitest";

import type { ModStudioSdkSchemaSnapshot } from "@/lib/tauri/mod-studio-contract";

import {
  MOD_SOURCE_COMPLETION_TRIGGER_CHARACTERS,
  MOD_SOURCE_SUGGEST_OPTIONS,
} from "./mod-source-editor-options";

import {
  modSourceCompletion,
  modSourceHover,
  modSourceOccurrences,
  modSourceSignatureHelp,
} from "./mod-source-intelligence";

const schema: ModStudioSdkSchemaSnapshot = {
  contractVersion: 10,
  schemaVersion: 1,
  symbols: [
    {
      label: "nte::memory::read_ptr(base, offset)",
      insertText: "nte::memory::read_ptr(",
      kind: "function",
      returnType: "std::uintptr_t",
      documentationKey: "Checked memory primitive.",
    },
    {
      label: 'nte::ipc::emit("event", value...)',
      insertText: 'nte::ipc::emit("event.name", ',
      kind: "function",
      returnType: "bool",
      documentationKey: "Publishes Mod IPC data.",
    },
  ],
};

describe("Mod source intelligence", () => {
  it("keeps automatic completion enabled for code and namespace triggers", () => {
    expect(MOD_SOURCE_SUGGEST_OPTIONS).toMatchObject({
      quickSuggestions: {
        other: true,
        comments: false,
        strings: false,
      },
      suggestOnTriggerCharacters: true,
      wordBasedSuggestions: "off",
    });
    expect(MOD_SOURCE_COMPLETION_TRIGGER_CHARACTERS).toEqual([":", "."]);
  });

  it("filters schema completions and replaces only the active token", () => {
    const source = "auto value = nte::memory::read_ + suffix;";
    const cursor = source.indexOf(" + suffix");
    const completion = modSourceCompletion(schema, source, cursor, false);

    expect(completion?.items[0]?.label).toBe(
      "nte::memory::read_ptr(base, offset)",
    );
    expect(source.slice(completion!.rangeStart, completion!.rangeEnd)).toBe(
      "nte::memory::read_",
    );
  });

  it("includes functions and parameters declared in the active document", () => {
    const source =
      "std::uint64_t sample(const std::uint64_t player_state) {\n    play\n}";
    const cursor = source.indexOf("play") + 4;
    const completion = modSourceCompletion(schema, source, cursor, false);

    expect(completion?.items.map((item) => item.label)).toContain(
      "player_state",
    );
  });

  it("keeps completion closed inside comments and string literals", () => {
    expect(
      modSourceCompletion(schema, "// nte::memory::read_", 21, false),
    ).toBeNull();
    expect(
      modSourceCompletion(schema, '"nte::memory::read_', 19, false),
    ).toBeNull();
  });

  it("tracks nested calls and the active signature parameter", () => {
    const source = 'nte::ipc::emit("post.value", helper(1, 2), ';
    const signature = modSourceSignatureHelp(schema, source, source.length);

    expect(signature).toMatchObject({
      activeParameter: 1,
      parameters: ['"event"', "value..."],
    });
  });

  it("describes an SDK function only when its symbol is hovered", () => {
    const source = "auto value = nte::memory::read_ptr(base, offset);";
    const offset = source.indexOf("read_ptr") + 3;
    const hover = modSourceHover(schema, source, offset);

    expect(hover).toMatchObject({
      label: "nte::memory::read_ptr(base, offset)",
      detail: "nte::memory::read_ptr(base, offset) -> std::uintptr_t",
      documentationKey: "Checked memory primitive.",
    });
    expect(source.slice(hover!.rangeStart, hover!.rangeEnd)).toBe(
      "nte::memory::read_ptr",
    );
  });

  it("describes document variables and suppresses hover inside comments", () => {
    const source =
      "std::uintptr_t sample(std::uintptr_t player_controller) {\n" +
      "    return player_controller;\n" +
      "}";
    const offset = source.lastIndexOf("player_controller") + 4;

    expect(modSourceHover(schema, source, offset)).toMatchObject({
      label: "player_controller",
      detail: "player_controller: std::uintptr_t",
      documentationKey: "Variable declared in this Mod source file.",
    });
    expect(
      modSourceHover(
        schema,
        "// nte::memory::read_ptr",
        "// nte::memory::read_ptr".length - 3,
      ),
    ).toBeNull();
  });

  it("highlights exact code occurrences of the hovered symbol", () => {
    const source =
      "std::uintptr_t player_controller = 0;\n" +
      "player_controller = player_controller_backup;\n" +
      "// player_controller";
    const offset = source.indexOf("player_controller") + 3;
    const occurrences = modSourceOccurrences(schema, source, offset);

    expect(
      occurrences.map(({ rangeStart, rangeEnd }) =>
        source.slice(rangeStart, rangeEnd),
      ),
    ).toEqual(["player_controller", "player_controller"]);
  });
});
