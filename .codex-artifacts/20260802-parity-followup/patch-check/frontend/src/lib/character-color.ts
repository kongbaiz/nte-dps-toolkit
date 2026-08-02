const CHARACTER_FALLBACK_COLORS = [
  "#2563eb",
  "#7c3aed",
  "#059669",
  "#c2410c",
  "#be185d",
  "#0891b2",
  "#a16207",
  "#be3741",
] as const;

function deterministicCharacterColorIndex(characterId: number): number {
  const id = characterId >>> 0;
  let hash = 0xcbf29ce484222325n;
  for (let shift = 0; shift < 32; shift += 8) {
    const byte = (id >>> shift) & 0xff;
    hash = BigInt.asUintN(64, (hash ^ BigInt(byte)) * 0x100000001b3n);
  }
  return Number(hash % BigInt(CHARACTER_FALLBACK_COLORS.length));
}

export function characterAccent(
  characterId: number,
  configuredColor: string | null,
): string {
  const candidate = configuredColor?.trim();
  if (candidate !== undefined && /^#[0-9a-f]{6}$/i.test(candidate)) {
    return candidate;
  }
  return CHARACTER_FALLBACK_COLORS[
    deterministicCharacterColorIndex(characterId)
  ];
}
