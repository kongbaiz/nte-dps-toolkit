import type { SkillsCharacterSnapshot } from "@/lib/tauri/generated/SkillsCharacterSnapshot";
import type { SkillsRowSnapshot } from "@/lib/tauri/generated/SkillsRowSnapshot";
import type { SkillsSnapshot } from "@/lib/tauri/generated/SkillsSnapshot";

export interface SkillCharacterSummary extends SkillsCharacterSnapshot {
  share: number;
}

export interface SkillViewMetrics {
  damage: number;
  entries: number;
}

export function buildSkillCharacterSummaries(
  snapshot: SkillsSnapshot,
): SkillCharacterSummary[] {
  return snapshot.characters.map((character) => ({
    ...character,
    share: skillDamageShare(character.damage, snapshot.totalDamage),
  }));
}

export function skillRowsForCharacter(
  snapshot: SkillsSnapshot,
  characterId: number | null,
): SkillsRowSnapshot[] {
  return snapshot.rows
    .filter((row) => characterId === null || row.characterId === characterId)
    .toSorted(
      (left, right) =>
        right.damage - left.damage || left.name.localeCompare(right.name),
    );
}

export function skillViewMetrics(
  rows: readonly SkillsRowSnapshot[],
): SkillViewMetrics {
  return {
    damage: rows.reduce((sum, row) => sum + row.damage, 0),
    entries: rows.length,
  };
}

export function skillDamageShare(damage: number, totalDamage: number): number {
  return totalDamage > 0 ? damage / totalDamage : 0;
}
