export interface ModSourceIdentifierToken {
  text: string;
  offset: number;
  end: number;
  line: number;
  start: number;
}

export function stripCppLineComment(line: string): string {
  let cursor = 0;
  while (cursor < line.length) {
    const rawEnd = rawStringLiteralEnd(line, cursor);
    if (rawEnd !== null) {
      cursor = rawEnd;
      continue;
    }
    const character = line[cursor];
    if (character === '"' || character === "'") {
      cursor = quotedLiteralEnd(line, cursor, character);
      continue;
    }
    if (character === "/" && line[cursor + 1] === "*") {
      const end = line.indexOf("*/", cursor + 2);
      cursor = end < 0 ? line.length : end + 2;
      continue;
    }
    if (character === "/" && line[cursor + 1] === "/") {
      return line.slice(0, cursor);
    }
    cursor += 1;
  }
  return line;
}

export function scanCppIdentifiers(source: string): ModSourceIdentifierToken[] {
  const identifiers: ModSourceIdentifierToken[] = [];
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
  const advanceTo = (target: number): void => {
    while (offset < target) {
      advance();
    }
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
    const rawEnd = rawStringLiteralEnd(source, offset);
    if (rawEnd !== null) {
      advanceTo(rawEnd);
      continue;
    }
    if (character === '"' || character === "'") {
      advanceTo(quotedLiteralEnd(source, offset, character));
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

export function cppParenthesizedContent(
  source: string,
  openOffset: number,
): string | null {
  if (source[openOffset] !== "(") {
    return null;
  }
  let cursor = openOffset;
  let depth = 0;
  while (cursor < source.length) {
    const rawEnd = rawStringLiteralEnd(source, cursor);
    if (rawEnd !== null) {
      cursor = rawEnd;
      continue;
    }
    const character = source[cursor];
    if (character === '"' || character === "'") {
      cursor = quotedLiteralEnd(source, cursor, character);
      continue;
    }
    if (character === "/" && source[cursor + 1] === "/") {
      const end = source.indexOf("\n", cursor + 2);
      cursor = end < 0 ? source.length : end;
      continue;
    }
    if (character === "/" && source[cursor + 1] === "*") {
      const end = source.indexOf("*/", cursor + 2);
      cursor = end < 0 ? source.length : end + 2;
      continue;
    }
    if (character === "(") {
      depth += 1;
    } else if (character === ")") {
      depth -= 1;
      if (depth === 0) {
        return source.slice(openOffset + 1, cursor);
      }
    }
    cursor += 1;
  }
  return source.slice(openOffset + 1);
}

function quotedLiteralEnd(
  source: string,
  start: number,
  quote: string,
): number {
  let cursor = start + 1;
  let escaped = false;
  while (cursor < source.length) {
    const character = source[cursor];
    cursor += 1;
    if (escaped) {
      escaped = false;
    } else if (character === "\\") {
      escaped = true;
    } else if (character === quote) {
      break;
    }
  }
  return cursor;
}

function rawStringLiteralEnd(source: string, start: number): number | null {
  if (source[start] !== "R" || source[start + 1] !== '"') {
    return null;
  }
  const delimiterEnd = source.indexOf("(", start + 2);
  if (delimiterEnd < 0 || delimiterEnd - (start + 2) > 16) {
    return null;
  }
  const delimiter = source.slice(start + 2, delimiterEnd);
  if (/[/\\()\s]/.test(delimiter)) {
    return null;
  }
  const closing = `)${delimiter}"`;
  const closingOffset = source.indexOf(closing, delimiterEnd + 1);
  return closingOffset < 0 ? source.length : closingOffset + closing.length;
}
