import type { SkillsRow } from "@/lib/tauri/skills-contract";

export interface SkillTechnicalDetail {
  label: "GA" | "GE" | "GE Index";
  value: string;
}

export interface ClipboardWriter {
  writeText(value: string): Promise<void>;
}

export function skillTechnicalDetails(
  row: Pick<
    SkillsRow,
    "abilityName" | "gameplayEffectName" | "gameplayEffectIndex"
  >,
): SkillTechnicalDetail[] {
  const details: SkillTechnicalDetail[] = [];
  if (row.abilityName) details.push({ label: "GA", value: row.abilityName });
  if (row.gameplayEffectName) {
    details.push({ label: "GE", value: row.gameplayEffectName });
  }
  if (row.gameplayEffectIndex !== null) {
    details.push({
      label: "GE Index",
      value: String(row.gameplayEffectIndex),
    });
  }
  return details;
}

export function copyGameplayEffectIndex(
  index: number,
  clipboard: ClipboardWriter = navigator.clipboard,
): Promise<void> {
  return clipboard.writeText(String(index));
}
