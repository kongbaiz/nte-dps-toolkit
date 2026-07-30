export type ModSourceTokenKind =
  | "plain"
  | "comment"
  | "directive"
  | "macro"
  | "keyword"
  | "type"
  | "string"
  | "number"
  | "namespace"
  | "function";

export interface ModSourceToken {
  text: string;
  kind: ModSourceTokenKind;
}

export interface ModSourceLine {
  number: number;
  tokens: ModSourceToken[];
}

const KEYWORDS = new Set([
  "alignof",
  "auto",
  "bool",
  "break",
  "case",
  "catch",
  "class",
  "const",
  "constexpr",
  "continue",
  "default",
  "delete",
  "do",
  "else",
  "enum",
  "explicit",
  "false",
  "for",
  "if",
  "namespace",
  "new",
  "noexcept",
  "nullptr",
  "private",
  "protected",
  "public",
  "return",
  "sizeof",
  "static",
  "struct",
  "switch",
  "template",
  "this",
  "throw",
  "true",
  "try",
  "using",
  "virtual",
  "void",
  "while",
]);

const TYPES = new Set([
  "char",
  "double",
  "float",
  "int",
  "int8_t",
  "int16_t",
  "int32_t",
  "int64_t",
  "long",
  "short",
  "size_t",
  "uint8_t",
  "uint16_t",
  "uint32_t",
  "uint64_t",
  "unsigned",
]);

export function highlightModSource(source: string): ModSourceLine[] {
  const lines = source.split("\n");
  let insideBlockComment = false;

  return lines.map((line, index) => {
    const result = tokenizeLine(line, insideBlockComment);
    insideBlockComment = result.insideBlockComment;
    return {
      number: index + 1,
      tokens: result.tokens,
    };
  });
}

interface TokenizeLineResult {
  tokens: ModSourceToken[];
  insideBlockComment: boolean;
}

function tokenizeLine(
  line: string,
  startsInsideBlockComment: boolean,
): TokenizeLineResult {
  const tokens: ModSourceToken[] = [];
  let cursor = 0;
  let insideBlockComment = startsInsideBlockComment;

  while (cursor < line.length) {
    if (insideBlockComment) {
      const end = line.indexOf("*/", cursor);
      if (end === -1) {
        pushToken(tokens, line.slice(cursor), "comment");
        cursor = line.length;
        continue;
      }
      pushToken(tokens, line.slice(cursor, end + 2), "comment");
      cursor = end + 2;
      insideBlockComment = false;
      continue;
    }

    if (line.startsWith("//", cursor)) {
      pushToken(tokens, line.slice(cursor), "comment");
      break;
    }

    if (line.startsWith("/*", cursor)) {
      const end = line.indexOf("*/", cursor + 2);
      if (end === -1) {
        pushToken(tokens, line.slice(cursor), "comment");
        insideBlockComment = true;
        break;
      }
      pushToken(tokens, line.slice(cursor, end + 2), "comment");
      cursor = end + 2;
      continue;
    }

    const character = line[cursor];
    if (character === '"' || character === "'") {
      const end = stringEnd(line, cursor, character);
      pushToken(tokens, line.slice(cursor, end), "string");
      cursor = end;
      continue;
    }

    if (character === "#" && line.slice(0, cursor).trim() === "") {
      const match = /^#[A-Za-z_]+/.exec(line.slice(cursor));
      if (match !== null) {
        pushToken(tokens, match[0], "directive");
        cursor += match[0].length;
        continue;
      }
    }

    const number = /^(?:0[xX][0-9a-fA-F]+|\d+(?:\.\d+)?)/.exec(
      line.slice(cursor),
    );
    if (number !== null) {
      pushToken(tokens, number[0], "number");
      cursor += number[0].length;
      continue;
    }

    const identifier = /^[A-Za-z_][A-Za-z0-9_]*/.exec(line.slice(cursor));
    if (identifier !== null) {
      const text = identifier[0];
      pushToken(tokens, text, identifierKind(line, cursor, text));
      cursor += text.length;
      continue;
    }

    pushToken(tokens, character, "plain");
    cursor += 1;
  }

  if (line.length === 0) {
    pushToken(tokens, "", "plain");
  }

  return { tokens, insideBlockComment };
}

function identifierKind(
  line: string,
  cursor: number,
  identifier: string,
): ModSourceTokenKind {
  if (identifier.startsWith("NTE_")) {
    return "macro";
  }
  if (KEYWORDS.has(identifier)) {
    return "keyword";
  }
  if (TYPES.has(identifier)) {
    return "type";
  }
  if (identifier === "std" || identifier === "nte") {
    return "namespace";
  }
  const suffix = line.slice(cursor + identifier.length);
  if (/^\s*\(/.test(suffix)) {
    return "function";
  }
  return "plain";
}

function stringEnd(line: string, start: number, quote: string): number {
  let cursor = start + 1;
  while (cursor < line.length) {
    if (line[cursor] === "\\") {
      cursor += 2;
      continue;
    }
    cursor += 1;
    if (line[cursor - 1] === quote) {
      break;
    }
  }
  return Math.min(cursor, line.length);
}

function pushToken(
  tokens: ModSourceToken[],
  text: string,
  kind: ModSourceTokenKind,
) {
  const previous = tokens.at(-1);
  if (previous?.kind === kind) {
    previous.text += text;
  } else {
    tokens.push({ text, kind });
  }
}
