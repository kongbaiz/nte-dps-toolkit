const CHARACTER_FALLBACK_COLORS = [
  "#e8174b",
  "#17e817",
  "#e05cd5",
  "#17c5e8",
  "#e84b17",
  "#1717e8",
  "#1fa361",
  "#1154b0",
  "#a3561f",
  "#b4e05c",
  "#a31f61",
  "#611fa3",
  "#4feed3",
  "#e8b417",
  "#c517e8",
  "#4fee76",
  "#117bb0",
  "#e8e817",
  "#40a31f",
  "#e0a95c",
  "#174be8",
  "#1fa398",
  "#e81791",
  "#e0675c",
  "#a3a31f",
  "#e88017",
  "#91e817",
  "#17e8a2",
  "#4f91ee",
  "#a31f35",
  "#a31f8d",
  "#9e4fee",
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
