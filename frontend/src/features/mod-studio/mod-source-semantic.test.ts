import { describe, expect, it } from "vitest";

import type { ModStudioSdkSchemaSnapshot } from "@/lib/tauri/mod-studio-contract";

import {
  modSourceSemanticTokens,
  type ModSourceSemanticTokenKind,
} from "./mod-source-semantic";

const schema: ModStudioSdkSchemaSnapshot = {
  contractVersion: 10,
  schemaVersion: 1,
  symbols: [
    {
      label: "nte::game::player_controller",
      insertText: "nte::game::player_controller",
      kind: "property",
      returnType: "std::uintptr_t",
      documentationKey: "Current player controller.",
    },
  ],
};

describe("Mod source semantic tokens", () => {
  it("classifies macros, declarations, parameters, namespaces, and functions", () => {
    const source = [
      "NTE_SCRIPT(5);",
      'NTE_REQUIRES("game.session");',
      "std::uint64_t initialized = 0;",
      "void on_viewport_tick(const nte::viewport_tick_event& event)",
      "{",
      "    const auto player_controller = nte::game::player_controller;",
      "    std::uint64_t pause_mask = 0;",
      "    pause_mask = nte::combat_clock::pause_mask(player_controller);",
      "}",
    ].join("\n");
    const grouped = groupedTokenText(source);

    expect(grouped.macro).toEqual(["NTE_SCRIPT", "NTE_REQUIRES"]);
    expect(grouped.namespace).toEqual([
      "std",
      "nte",
      "nte",
      "game",
      "std",
      "nte",
      "combat_clock",
    ]);
    expect(grouped.type).toEqual([
      "uint64_t",
      "viewport_tick_event",
      "uint64_t",
    ]);
    expect(grouped.function).toEqual(["on_viewport_tick", "pause_mask"]);
    expect(grouped.variable).toEqual([
      "initialized",
      "event",
      "player_controller",
      "pause_mask",
      "pause_mask",
      "player_controller",
    ]);
    expect(grouped.property).toEqual(["player_controller"]);
  });

  it("does not color identifiers inside comments, strings, or directives", () => {
    const source = [
      "#include <nte/mod.hpp>",
      "// NTE_SCRIPT player_controller nte::game::player_controller",
      'const char* value = "NTE_SCRIPT player_controller";',
      'const char* raw = R"tag(NTE_SCRIPT player_controller // comment)tag";',
    ].join("\n");
    const text = Object.values(groupedTokenText(source)).flat();

    expect(text).not.toContain("NTE_SCRIPT");
    expect(text).not.toContain("nte");
    expect(text).not.toContain("player_controller");
  });

  function groupedTokenText(
    source: string,
  ): Record<ModSourceSemanticTokenKind, string[]> {
    const lines = source.split("\n");
    const grouped: Record<ModSourceSemanticTokenKind, string[]> = {
      macro: [],
      namespace: [],
      type: [],
      function: [],
      variable: [],
      property: [],
    };
    for (const token of modSourceSemanticTokens(source, schema)) {
      grouped[token.kind].push(
        lines[token.line].slice(token.start, token.start + token.length),
      );
    }
    return grouped;
  }
});
