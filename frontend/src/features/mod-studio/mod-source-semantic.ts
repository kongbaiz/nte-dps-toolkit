import type { ModStudioSdkSchemaSnapshot } from "@/lib/tauri/mod-studio-contract";

import { modSourceDocumentSymbols } from "./mod-source-intelligence";
import {
  scanCppIdentifiers,
  type ModSourceIdentifierToken,
} from "./mod-source-lexical";

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

  return scanCppIdentifiers(source).flatMap((identifier) => {
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
  identifier: ModSourceIdentifierToken,
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
