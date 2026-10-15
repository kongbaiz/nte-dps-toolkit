import type { ReleaseAction } from "@/lib/tauri/release-plugin-client";
export const RELEASE_GROUPS = [
  {
    id: "combat",
    pluginFile: "NTE_PluginCombat.dll",
    label: "Combat functions",
    description: "Adjust combat features and preview or export your reports.",
    actions: [
      "frameRateConfigure",
      "quickUltimateConfigure",
      "prebattleSkillUnlock",
      "combatReport",
      "combatExport",
    ],
    advanced: [
      "frameRateStatus",
      "frameRateRefresh",
      "quickUltimateStatus",
      "prebattleSkillStatus",
      "combatStatus",
      "combatOperation",
      "evidenceStatus",
      "runtimeRefresh",
      "radarRefresh",
      "traceEnable",
      "combatReset",
      "traceClear",
    ],
  },
  {
    id: "account",
    pluginFile: "NTE_PluginUser.dll",
    label: "Account data",
    description:
      "Read characters and inventory, then preview or export the frozen snapshot.",
    actions: ["userRefresh", "userSnapshot", "userExport"],
    advanced: ["userStatus", "userOperation", "userCancel"],
  },
  {
    id: "training",
    pluginFile: "NTE_PluginTraining.dll",
    label: "Training controls",
    description: "Use the Tab key in game to open the wooden dummy wheel.",
    actions: ["trainingWoodenEnable", "trainingWoodenDisable"],
    advanced: ["trainingStatus", "trainingOperation"],
  },
  {
    id: "host",
    pluginFile: null,
    label: "Host controls",
    description: "Check host compatibility and manage the runtime lifecycle.",
    actions: ["hostStatus", "shutdown"],
    advanced: [],
  },
] as const satisfies readonly {
  id: string;
  pluginFile: string | null;
  label: string;
  description: string;
  actions: readonly ReleaseAction[];
  advanced: readonly ReleaseAction[];
}[];
export type ReleaseGroupId = (typeof RELEASE_GROUPS)[number]["id"];

export type ReleaseControlGroup = (typeof RELEASE_GROUPS)[number];
export function pluginControlGroup(file: string) {
  return RELEASE_GROUPS.find(
    (group) => group.pluginFile?.toLowerCase() === file.toLowerCase(),
  );
}
