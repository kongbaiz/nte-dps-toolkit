import { t, tf } from "@/lib/i18n";
import type { MainDpsRound } from "@/lib/tauri/main-dps-contract";

const numberFormatter = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 0,
});

export function formatMainMetric(value: number): string {
  return numberFormatter.format(Math.round(value));
}

export function formatDuration(seconds: number): string {
  return `${seconds.toFixed(1)}${t("s")}`;
}

export function damagePercent(damage: number, totalDamage: number): number {
  return totalDamage > 0 ? (damage / totalDamage) * 100 : 0;
}

const characterFallbackColors = [
  "#b92f59",
  "#554078",
  "#76607f",
  "#8c7f88",
  "#2f7d6d",
  "#476c91",
] as const;

export function characterAccent(
  characterId: number,
  configuredColor: string | null,
): string {
  const candidate = configuredColor?.trim();
  if (candidate !== undefined && /^#[0-9a-f]{6}$/i.test(candidate)) {
    return candidate;
  }
  return characterFallbackColors[
    Math.abs(characterId) % characterFallbackColors.length
  ];
}

export function roundLabel(round: MainDpsRound): string {
  if (round.live) return t("Live");
  const prefix =
    round.abyssFloor === null
      ? t("Archived round")
      : tf("Abyss Floor {}", [String(round.abyssFloor)]);
  return round.displayTime === null
    ? prefix
    : `${prefix} · ${round.displayTime}`;
}

export function isGenerationNewer(
  candidate: string,
  current: string | null,
): boolean {
  if (current === null) return true;
  try {
    return BigInt(candidate) > BigInt(current);
  } catch {
    return candidate !== current;
  }
}

export type MainCaptureStatusTone = "idle" | "active" | "transition" | "error";

export function mainCaptureStatusTone(phase: string): MainCaptureStatusTone {
  if (phase === "running") return "active";
  if (phase === "starting" || phase === "stopping") return "transition";
  if (phase === "failed") return "error";
  return "idle";
}

export type MainCharacterListState =
  "rows" | "hidden" | "unattributed" | "combat-empty";

export function mainCharacterListState(
  totalDamage: number,
  sourceRows: number,
  visibleRows: number,
): MainCharacterListState {
  if (visibleRows > 0) return "rows";
  if (sourceRows > 0) return "hidden";
  if (totalDamage > 0) return "unattributed";
  return "combat-empty";
}

export interface FloatingPanelAnchor {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

export interface FloatingPanelViewport {
  width: number;
  height: number;
}

export function appearancePanelPosition(
  anchor: FloatingPanelAnchor,
  viewport: FloatingPanelViewport,
  panel = { width: 176, height: 160 },
): { left: number; top: number } {
  const margin = 8;
  const gap = 6;
  const maxLeft = Math.max(margin, viewport.width - panel.width - margin);
  const left = Math.min(Math.max(margin, anchor.right - panel.width), maxLeft);
  const below = anchor.bottom + gap;
  const above = anchor.top - gap - panel.height;
  const maxTop = Math.max(margin, viewport.height - panel.height - margin);
  const top =
    below + panel.height <= viewport.height - margin
      ? below
      : above >= margin
        ? above
        : Math.min(Math.max(margin, below), maxTop);

  return { left, top };
}
