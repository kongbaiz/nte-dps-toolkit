import type { ModStudioSdkSchemaSnapshot } from "@/lib/tauri/mod-studio-contract";

import { modSourceDocumentSymbols } from "./mod-source-intelligence";

export const MOD_SOURCE_SEMANTIC_TOKEN_TYPES = [
  "macro",
  "namespace",
  "type",
  "function",
  "variable",
  "property",
] as const;

export type ModSourceSemanticTokenKind =
  (typeof MOD_SOURCE_SEMANTIC_TOKEN_TYPES)[number];

export interface ModSourceSemanticToken {
  line: number;
  start: number;
  length: number;
  kind: ModSourceSemanticTokenKind;
}

interface IdentifierToken {
  text: string;
  offset: number;
  end: number;
  line: number;
  start: number;
}

const TYPE_NAMES = new Set([
  "array",
  "bool",
  "byte",
  "char",
  "double",
  "float",
  "int",
  "int8_t",
  "int16_t",
  "int32_t",
  "int64_t",
  "long",
  "optional",
  "ptrdiff_t",
  "short",
  "size_t",
  "span",
  "string",
  "string_view",
  "uint8_t",
  "uint16_t",
  "uint32_t",
  "uint64_t",
  "uintptr_t",
  "unsigned",
  "vector",
]);

export function modSourceSemanticTokens(
  source: string,
  schema: ModStudioSdkSchemaSnapshot | null,
): ModSourceSemanticToken[] {
  const documentSymbols = modSourceDocumentSymbols(source);
  const variables = new Set(
    documentSymbols
      .filter((symbol) => symbol.kind === "variable")
      .map((symbol) => symbol.label),
  );
  const sdkProperties = new Set(
    (schema?.symbols ?? [])
      .filter((symbol) => symbol.kind === "property")
      .map((symbol) => symbol.label.split(/[.:]/).at(-1) ?? symbol.label),
  );

  return scanIdentifiers(source).flatMap((identifier) => {
    const kind = semanticKind(source, identifier, variables, sdkProperties);
    return kind === null
      ? []
      : [
          {
            line: identifier.line,
            start: identifier.start,
            length: identifier.text.length,
            kind,
          },
        ];
  });
}

function semanticKind(
  source: string,
  identifier: IdentifierToken,
  variables: ReadonlySet<string>,
  sdkProperties: ReadonlySet<string>,
): ModSourceSemanticTokenKind | null {
  if (/^NTE_[A-Z0-9_]+$/.test(identifier.text)) {
    return "macro";
  }
  if (followedBy(source, identifier.end, "(")) {
    return "function";
  }
  if (followedBy(source, identifier.end, "::")) {
    return "namespace";
  }
  if (TYPE_NAMES.has(identifier.text) || identifier.text.endsWith("_event")) {
    return "type";
  }
  if (
    precededBy(source, identifier.offset, "::") &&
    sdkProperties.has(identifier.text)
  ) {
    return "property";
  }
  return variables.has(identifier.text) ? "variable" : null;
}

function followedBy(source: string, offset: number, expected: string): boolean {
  return source
    .slice(skipWhitespaceForward(source, offset))
    .startsWith(expected);
}

function precededBy(source: string, offset: number, expected: string): boolean {
  const end = skipWhitespaceBackward(source, offset);
  return source.slice(Math.max(0, end - expected.length), end) === expected;
}

function skipWhitespaceForward(source: string, offset: number): number {
  let cursor = offset;
  while (cursor < source.length && /\s/.test(source[cursor])) {
    cursor += 1;
  }
  return cursor;
}

function skipWhitespaceBackward(source: string, offset: number): number {
  let cursor = offset;
  while (cursor > 0 && /\s/.test(source[cursor - 1])) {
    cursor -= 1;
  }
  return cursor;
}

function scanIdentifiers(source: string): IdentifierToken[] {
  const identifiers: IdentifierToken[] = [];
  let offset = 0;
  let line = 0;
  let column = 0;

  const advance = (): string => {
    const character = source[offset] ?? "";
    offset += 1;
    if (character === "\n") {
      line += 1;
      column = 0;
    } else {
      column += 1;
    }
    return character;
  };

  while (offset < source.length) {
    const character = source[offset];
    const next = source[offset + 1];
    if (
      character === "#" &&
      source.slice(source.lastIndexOf("\n", offset - 1) + 1, offset).trim()
        .length === 0
    ) {
      while (offset < source.length && source[offset] !== "\n") {
        advance();
      }
      continue;
    }
    if (character === "/" && next === "/") {
      while (offset < source.length && source[offset] !== "\n") {
        advance();
      }
      continue;
    }
    if (character === "/" && next === "*") {
      advance();
      advance();
      while (
        offset < source.length &&
        !(source[offset] === "*" && source[offset + 1] === "/")
      ) {
        advance();
      }
      if (offset < source.length) {
        advance();
        advance();
      }
      continue;
    }
    if (character === '"' || character === "'") {
      const quote = advance();
      let escaped = false;
      while (offset < source.length) {
        const current = advance();
        if (escaped) {
          escaped = false;
        } else if (current === "\\") {
          escaped = true;
        } else if (current === quote) {
          break;
        }
      }
      continue;
    }
    if (/[A-Za-z_]/.test(character)) {
      const startOffset = offset;
      const startLine = line;
      const startColumn = column;
      advance();
      while (offset < source.length && /[A-Za-z0-9_]/.test(source[offset])) {
        advance();
      }
      identifiers.push({
        text: source.slice(startOffset, offset),
        offset: startOffset,
        end: offset,
        line: startLine,
        start: startColumn,
      });
      continue;
    }
    advance();
  }

  return identifiers;
}
