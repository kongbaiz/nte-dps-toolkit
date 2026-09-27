import art from "@res/data/characters/panel_art.json";
import equipment from "@res/data/equipment/equipment.json";
import characters from "@res/data/characters/characters.json";
import { currentFrontendLanguage, t } from "@/lib/i18n";
import type { CharacterEntry } from "@/lib/tauri/user-characters-client";

const images = import.meta.glob<string>(
  [
    "@res/images/character-panel/**/*.png",
    "@res/images/kongmu/256/*.png",
    "@res/images/fangkuai/*.png",
    "@res/images/characters/player*.png",
  ],
  { eager: true, query: "?url", import: "default" },
);
const byPath = new Map(
  Object.entries(images).map(([path, url]) => [
    path.slice(path.indexOf("/res/") + 1),
    url,
  ]),
);
// Arc IDs are Unreal FNames: the live spelling can differ in case from the
// exported data-table key (for example fork_Wushoutieyu).
const arcPaths = new Map(
  Object.entries(art.arcs).map(([id, path]) => [id.toLowerCase(), path]),
);
const lookup = <T>(
  map: Record<string, T>,
  key: string | null,
): T | undefined =>
  key === null || !Object.hasOwn(map, key) ? undefined : map[key];
export function panelImage(
  kind: "character" | "avatar" | "arc" | "equipment",
  id: string | null,
): string | null {
  const path =
    kind === "avatar"
      ? lookup(characters.characters, id)?.avatar
      : kind === "character"
        ? (lookup(art.characters, id) ??
          lookup(characters.characters, id)?.avatar)
        : kind === "arc"
          ? id === null
            ? null
            : arcPaths.get(id.toLowerCase())
          : lookup(equipment.items, id)?.icon;
  return path ? (byPath.get(path) ?? null) : null;
}
export const entryValue = (
  entry: CharacterEntry,
  label: string,
): string | null => entry.fields.find((f) => f.label === label)?.value ?? null;
export function attributeLabel(key: string): string {
  const attribute = lookup(equipment.attributes, key);
  if (!attribute) return t(key);
  const lang = currentFrontendLanguage();
  return lang === "zh-CN"
    ? attribute.name_zh
    : lang === "ja"
      ? attribute.name_ja
      : attribute.name_en;
}
const plain = new Intl.NumberFormat("en-US", {
  useGrouping: false,
  maximumFractionDigits: 2,
});
const attack = new Intl.NumberFormat("en-US", {
  useGrouping: false,
  maximumFractionDigits: 1,
});
const integer = new Intl.NumberFormat("en-US", {
  useGrouping: false,
  maximumFractionDigits: 0,
});
// Presentation only: never change the read model or use rounded values in calculations.
export function formatPanelNumber(
  raw: string | null,
  kind: "plain" | "attack" | "integer" | "percent" | "score" = "plain",
  bonus = false,
): string {
  if (raw === null) return "—";
  if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(raw)) return raw;
  const value = Number(raw);
  if (!Number.isFinite(value)) return raw;
  // Identifiers and integers larger than the JS exact range must not be rounded.
  if (Math.abs(value) > Number.MAX_SAFE_INTEGER) return raw;
  const scaled = kind === "percent" ? value * 100 : value;
  if (!Number.isFinite(scaled)) return raw;
  let number =
    kind === "score"
      ? scaled.toFixed(1)
      : kind === "attack"
        ? attack.format(scaled)
        : kind === "integer"
          ? integer.format(scaled)
          : plain.format(scaled);
  if (Number(number) === 0 && number.startsWith("-")) number = number.slice(1);
  return `${bonus && Number(number) > 0 ? "+" : ""}${number}${kind === "percent" ? "%" : ""}`;
}
export function formatModifier(label: string, raw: string | null): string {
  const definition = lookup(equipment.attributes, label);
  // Exact property-ID match only, reusing the equipment catalog's unit definition.
  // Unknown properties never acquire a percentage sign or a guessed stat name.
  return formatPanelNumber(
    raw,
    definition?.percent ? "percent" : "plain",
    definition !== undefined,
  );
}
export function isKnownModifier(label: string): boolean {
  return lookup(equipment.attributes, label) !== undefined;
}
export function equipmentName(id: string | null): string | null {
  const item = lookup(equipment.items, id);
  if (!item) return null;
  const lang = currentFrontendLanguage();
  return lang === "zh-CN"
    ? item.name_zh
    : lang === "ja"
      ? item.name_ja
      : item.name_en;
}
