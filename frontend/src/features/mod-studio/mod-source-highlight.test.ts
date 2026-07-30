import { describe, expect, it } from "vitest";

import { highlightModSource } from "./mod-source-highlight";

describe("highlightModSource", () => {
  it("keeps line numbers stable and classifies the NTE declarations", () => {
    const lines = highlightModSource(
      '#include <nte/mod.hpp>\nNTE_MOD("combat-clock");\n',
    );

    expect(lines.map((line) => line.number)).toEqual([1, 2, 3]);
    expect(lines[0].tokens).toContainEqual({
      text: "#include",
      kind: "directive",
    });
    expect(lines[1].tokens).toContainEqual({
      text: "NTE_MOD",
      kind: "macro",
    });
    expect(lines[1].tokens).toContainEqual({
      text: '"combat-clock"',
      kind: "string",
    });
  });

  it("carries block-comment state across source lines", () => {
    const lines = highlightModSource(
      "const auto value = 1; /* note\ncontinued */ return value;",
    );

    expect(lines[0].tokens.at(-1)).toEqual({
      text: "/* note",
      kind: "comment",
    });
    expect(lines[1].tokens[0]).toEqual({
      text: "continued */",
      kind: "comment",
    });
    expect(lines[1].tokens).toContainEqual({
      text: "return",
      kind: "keyword",
    });
  });

  it("does not interpret comment markers inside strings", () => {
    const [line] = highlightModSource(
      'nte::log("https://TARGET/path"); // trailing',
    );

    expect(line.tokens).toContainEqual({
      text: '"https://TARGET/path"',
      kind: "string",
    });
    expect(line.tokens.at(-1)).toEqual({
      text: "// trailing",
      kind: "comment",
    });
  });
});
