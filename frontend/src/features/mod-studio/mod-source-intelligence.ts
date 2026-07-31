import type {
  ModStudioSdkSchemaSnapshot,
  ModStudioSdkSymbol,
  ModStudioSdkSymbolKind,
} from "@/lib/tauri/mod-studio-contract";

export type ModSourceCompletionKind = ModStudioSdkSymbolKind | "variable";

export interface ModSourceCompletionItem {
  label: string;
  insertText: string;
  kind: ModSourceCompletionKind;
  returnType: string | null;
  documentationKey: string;
}

export interface ModSourceCompletionResult {
  rangeStart: number;
  rangeEnd: number;
  query: string;
  items: ModSourceCompletionItem[];
}

export interface ModSourceSignatureHelp {
  label: string;
  parameters: string[];
  activeParameter: number;
  documentationKey: string;
}

export interface ModSourceHover {
  rangeStart: number;
  rangeEnd: number;
  label: string;
  detail: string;
  documentationKey: string;
}

export interface ModSourceRange {
  rangeStart: number;
  rangeEnd: number;
}

const TOKEN_CHARACTER = /[A-Za-z0-9_:#.]/;
const QUALIFIED_SYMBOL_CHARACTER = /[A-Za-z0-9_:]/;

export function modSourceCompletion(
  schema: ModStudioSdkSchemaSnapshot,
  source: string,
  cursor: number,
  explicit: boolean,
): ModSourceCompletionResult | null {
  if (!isCodePosition(source, cursor)) {
    return null;
  }
  const range = completionRange(source, cursor);
  if (range === null && !explicit) {
    return null;
  }
  const rangeStart = range?.start ?? cursor;
  const query = (range?.query ?? "").toLowerCase();
  const items = [
    ...schema.symbols.map(projectSdkSymbol),
    ...modSourceDocumentSymbols(source),
  ]
    .filter((item) => {
      if (query.length === 0) {
        return explicit;
      }
      return (
        item.label.toLowerCase().includes(query) ||
        item.insertText.toLowerCase().includes(query)
      );
    })
    .sort(
      (left, right) =>
        completionRank(left, query) - completionRank(right, query) ||
        left.label.localeCompare(right.label),
    );
  const seen = new Set<string>();
  return {
    rangeStart,
    rangeEnd: cursor,
    query,
    items: items.filter((item) => {
      if (seen.has(item.insertText)) {
        return false;
      }
      seen.add(item.insertText);
      return true;
    }),
  };
}

function isCodePosition(source: string, cursor: number): boolean {
  let inString = false;
  let inCharacter = false;
  let inLineComment = false;
  let inBlockComment = false;
  let escaped = false;
  for (let index = 0; index < cursor; index += 1) {
    const character = source[index];
    const next = source[index + 1];
    if (inLineComment) {
      if (character === "\n") {
        inLineComment = false;
      }
      continue;
    }
    if (inBlockComment) {
      if (character === "*" && next === "/") {
        index += 1;
        inBlockComment = false;
      }
      continue;
    }
    if (inString || inCharacter) {
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (
        (inString && character === '"') ||
        (inCharacter && character === "'")
      ) {
        inString = false;
        inCharacter = false;
      }
      continue;
    }
    if (character === "/" && next === "/") {
      index += 1;
      inLineComment = true;
    } else if (character === "/" && next === "*") {
      index += 1;
      inBlockComment = true;
    } else if (character === '"') {
      inString = true;
    } else if (character === "'") {
      inCharacter = true;
    }
  }
  return !inString && !inCharacter && !inLineComment && !inBlockComment;
}

export function modSourceSignatureHelp(
  schema: ModStudioSdkSchemaSnapshot,
  source: string,
  cursor: number,
): ModSourceSignatureHelp | null {
  const context = activeCallContext(source, cursor);
  if (context === null) {
    return null;
  }
  const item = [
    ...schema.symbols.map(projectSdkSymbol),
    ...modSourceDocumentSymbols(source),
  ].find(
    (candidate) =>
      candidate.kind === "function" &&
      candidate.label.startsWith(`${context.name}(`),
  );
  if (item === undefined) {
    return null;
  }
  const parameters = signatureParameters(item.label);
  return {
    label:
      item.returnType === null
        ? item.label
        : `${item.label} -> ${item.returnType}`,
    parameters,
    activeParameter: Math.min(
      context.activeParameter,
      Math.max(0, parameters.length - 1),
    ),
    documentationKey: item.documentationKey,
  };
}

export function modSourceHover(
  schema: ModStudioSdkSchemaSnapshot,
  source: string,
  offset: number,
): ModSourceHover | null {
  if (!isCodePosition(source, offset)) {
    return null;
  }
  const range = qualifiedSymbolRange(source, offset);
  if (range === null) {
    return null;
  }
  const symbolText = source.slice(range.start, range.end);
  const item = [
    ...schema.symbols.map(projectSdkSymbol),
    ...modSourceDocumentSymbols(source),
  ].find((candidate) => {
    const symbolName = candidate.label.split("(")[0];
    return (
      symbolName === symbolText || symbolName.split("::").at(-1) === symbolText
    );
  });
  if (item === undefined) {
    return null;
  }
  const detail =
    item.kind === "variable"
      ? `${item.label}: ${item.returnType ?? "auto"}`
      : item.returnType === null
        ? item.label
        : `${item.label} -> ${item.returnType}`;
  return {
    rangeStart: range.start,
    rangeEnd: range.end,
    label: item.label,
    detail,
    documentationKey: item.documentationKey,
  };
}

export function modSourceOccurrences(
  schema: ModStudioSdkSchemaSnapshot,
  source: string,
  offset: number,
): ModSourceRange[] {
  const hover = modSourceHover(schema, source, offset);
  if (hover === null) {
    return [];
  }
  const symbol = source.slice(hover.rangeStart, hover.rangeEnd);
  const ranges: ModSourceRange[] = [];
  let matchStart = source.indexOf(symbol);
  while (matchStart >= 0) {
    const matchEnd = matchStart + symbol.length;
    const leftBoundary =
      matchStart === 0 ||
      !QUALIFIED_SYMBOL_CHARACTER.test(source[matchStart - 1]);
    const rightBoundary =
      matchEnd === source.length ||
      !QUALIFIED_SYMBOL_CHARACTER.test(source[matchEnd]);
    if (leftBoundary && rightBoundary && isCodePosition(source, matchStart)) {
      ranges.push({ rangeStart: matchStart, rangeEnd: matchEnd });
    }
    matchStart = source.indexOf(symbol, matchStart + symbol.length);
  }
  return ranges;
}

function completionRange(
  source: string,
  cursor: number,
): { start: number; query: string } | null {
  let start = Math.min(Math.max(cursor, 0), source.length);
  while (start > 0 && TOKEN_CHARACTER.test(source[start - 1])) {
    start -= 1;
  }
  if (start === cursor) {
    return null;
  }
  return { start, query: source.slice(start, cursor) };
}

function qualifiedSymbolRange(
  source: string,
  offset: number,
): { start: number; end: number } | null {
  const boundedOffset = Math.min(Math.max(offset, 0), source.length);
  let start = boundedOffset;
  let end = boundedOffset;
  while (start > 0 && QUALIFIED_SYMBOL_CHARACTER.test(source[start - 1])) {
    start -= 1;
  }
  while (end < source.length && QUALIFIED_SYMBOL_CHARACTER.test(source[end])) {
    end += 1;
  }
  while (source.slice(start, start + 2) === "::") {
    start += 2;
  }
  while (source.slice(end - 2, end) === "::") {
    end -= 2;
  }
  return start === end ? null : { start, end };
}

function projectSdkSymbol(symbol: ModStudioSdkSymbol): ModSourceCompletionItem {
  return {
    label: symbol.label,
    insertText: symbol.insertText,
    kind: symbol.kind,
    returnType: symbol.returnType,
    documentationKey: symbol.documentationKey,
  };
}

function completionRank(item: ModSourceCompletionItem, query: string): number {
  const label = item.label.toLowerCase();
  const insert = item.insertText.toLowerCase();
  const prefixRank =
    label.startsWith(query) || insert.startsWith(query)
      ? 0
      : label.split(/[:.(\s]/).some((segment) => segment.startsWith(query))
        ? 10
        : 20;
  const kindRank: Record<ModSourceCompletionKind, number> = {
    variable: 0,
    function: 1,
    property: 2,
    declaration: 3,
    snippet: 4,
  };
  return prefixRank + kindRank[item.kind];
}

export function modSourceDocumentSymbols(
  source: string,
): ModSourceCompletionItem[] {
  const items: ModSourceCompletionItem[] = [];
  for (const originalLine of source.split("\n")) {
    const line = (originalLine.split("//")[0] ?? "").trim();
    if (line.length === 0 || line.startsWith("#") || line.startsWith("NTE_")) {
      continue;
    }
    const open = line.indexOf("(");
    if (open >= 0) {
      const prefix = line.slice(0, open).trim();
      if (
        !["if", "for", "while", "switch"].includes(prefix) &&
        !prefix.includes("=")
      ) {
        const declared = splitDeclaredSymbol(prefix);
        if (declared !== null) {
          const parameters = (line.slice(open + 1).split(")")[0] ?? "").trim();
          items.push({
            label: `${declared.name}(${parameters})`,
            insertText: `${declared.name}(`,
            kind: "function",
            returnType: declared.typeName,
            documentationKey: "Function declared in this Mod source file.",
          });
          for (const parameter of parameters
            .split(",")
            .map((value) => value.trim())) {
            const declaredParameter = splitDeclaredSymbol(parameter);
            if (declaredParameter !== null) {
              items.push(variableCompletion(declaredParameter));
            }
          }
        }
      }
    }
    const declaration = (line.replace(/[;{]+$/, "").split("=")[0] ?? "").trim();
    const declared = splitDeclaredSymbol(declaration);
    if (declared !== null) {
      items.push(variableCompletion(declared));
    }
  }
  return items;
}

function variableCompletion(declared: {
  typeName: string;
  name: string;
}): ModSourceCompletionItem {
  return {
    label: declared.name,
    insertText: declared.name,
    kind: "variable",
    returnType: declared.typeName,
    documentationKey: "Variable declared in this Mod source file.",
  };
}

function splitDeclaredSymbol(
  declaration: string,
): { typeName: string; name: string } | null {
  const match = /^(.*?)\s+([A-Za-z_][A-Za-z0-9_]*)\s*$/.exec(declaration);
  if (match === null) {
    return null;
  }
  const typeName = match[1].trim();
  const name = match[2].replace(/^[&*]+/, "");
  return typeName.length === 0 ? null : { typeName, name };
}

function activeCallContext(
  source: string,
  cursor: number,
): { name: string; activeParameter: number } | null {
  const parentheses: number[] = [];
  let inString = false;
  let inCharacter = false;
  let inLineComment = false;
  let inBlockComment = false;
  let escaped = false;
  for (let index = 0; index < cursor; index += 1) {
    const character = source[index];
    const next = source[index + 1];
    if (inLineComment) {
      if (character === "\n") {
        inLineComment = false;
      }
      continue;
    }
    if (inBlockComment) {
      if (character === "*" && next === "/") {
        index += 1;
        inBlockComment = false;
      }
      continue;
    }
    if (inString || inCharacter) {
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (
        (inString && character === '"') ||
        (inCharacter && character === "'")
      ) {
        inString = false;
        inCharacter = false;
      }
      continue;
    }
    if (character === "/" && next === "/") {
      index += 1;
      inLineComment = true;
    } else if (character === "/" && next === "*") {
      index += 1;
      inBlockComment = true;
    } else if (character === '"') {
      inString = true;
    } else if (character === "'") {
      inCharacter = true;
    } else if (character === "(") {
      parentheses.push(index);
    } else if (character === ")") {
      parentheses.pop();
    }
  }
  const open = parentheses.at(-1);
  if (open === undefined) {
    return null;
  }
  const nameEnd = source.slice(0, open).trimEnd().length;
  let nameStart = nameEnd;
  while (nameStart > 0 && /[A-Za-z0-9_:]/.test(source[nameStart - 1])) {
    nameStart -= 1;
  }
  const name = source.slice(nameStart, nameEnd);
  if (name.length === 0) {
    return null;
  }
  return {
    name,
    activeParameter: topLevelCommaCount(source.slice(open + 1, cursor)),
  };
}

function topLevelCommaCount(source: string): number {
  let depth = 0;
  let count = 0;
  let inString = false;
  let escaped = false;
  for (const character of source) {
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (character === '"') {
        inString = false;
      }
    } else if (character === '"') {
      inString = true;
    } else if (character === "(" || character === "[" || character === "{") {
      depth += 1;
    } else if (character === ")" || character === "]" || character === "}") {
      depth = Math.max(0, depth - 1);
    } else if (character === "," && depth === 0) {
      count += 1;
    }
  }
  return count;
}

function signatureParameters(label: string): string[] {
  const open = label.indexOf("(");
  const close = label.lastIndexOf(")");
  if (open < 0 || close <= open) {
    return [];
  }
  return label
    .slice(open + 1, close)
    .split(",")
    .map((parameter) => parameter.trim())
    .filter((parameter) => parameter.length > 0);
}
