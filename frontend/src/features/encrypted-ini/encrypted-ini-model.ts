export function findEncryptedIniMatches(text: string, query: string): number[] {
  const needle = query.trim();
  if (needle.length === 0) return [];
  const haystack = text.toLowerCase();
  const normalizedNeedle = needle.toLowerCase();
  const matches: number[] = [];
  let start = 0;
  while (start <= haystack.length - normalizedNeedle.length) {
    const index = haystack.indexOf(normalizedNeedle, start);
    if (index < 0) break;
    matches.push(index);
    start = index + Math.max(1, normalizedNeedle.length);
  }
  return matches;
}

export function nextEncryptedIniMatch(
  current: number | null,
  count: number,
): number | null {
  if (count === 0) return null;
  return current === null ? 0 : (current + 1) % count;
}

export function previousEncryptedIniMatch(
  current: number | null,
  count: number,
): number | null {
  if (count === 0) return null;
  return current === null ? count - 1 : (current - 1 + count) % count;
}

export function encryptedIniLineColumn(
  text: string,
  index: number,
): { line: number; column: number } {
  const prefix = text.slice(0, Math.max(0, index));
  const lines = prefix.split("\n");
  return { line: lines.length, column: (lines.at(-1)?.length ?? 0) + 1 };
}

export function encryptedIniCenteredScrollOffset(
  targetOffset: number,
  viewportExtent: number,
  contentExtent: number,
): number {
  const maximum = Math.max(0, contentExtent - viewportExtent);
  const centered = targetOffset - viewportExtent / 2;
  return Math.min(maximum, Math.max(0, centered));
}
