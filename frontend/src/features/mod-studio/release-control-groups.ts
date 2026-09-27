import type { ReleaseAction } from "@/lib/tauri/release-plugin-client";
export const RELEASE_GROUPS = [
  {
    id: "combat",
    pluginFile: "NTE_PluginCombat.dll",
    label: "Combat functions",
    description:
      "Review collection, inspect evidence and export native combat reports.",
    actions: [
      "prebattleSkillUnlock",
      "prebattleSkillStatus",
      "combatReport",
      "combatExport",
      "combatStatus",
      "combatOperation",
      "evidenceStatus",
      "runtimeRefresh",
      "radarRefresh",
      "traceEnable",
      "combatReset",
      "traceClear",
    ],
    advanced: [],
  },
  {
    id: "account",
    pluginFile: "NTE_PluginUser.dll",
    label: "Account data",
    description:
      "Read characters and inventory, then preview or export the frozen snapshot.",
    actions: [
      "userRefresh",
      "userSnapshot",
      "userExport",
      "userStatus",
      "userOperation",
      "userCancel",
    ],
    advanced: [],
  },
  {
    id: "network",
    pluginFile: "NTE_PluginNetwork.dll",
    label: "Network evidence",
    description:
      "Optional plugin-side diagnostics. This is not the DPS data source.",
    actions: ["networkStatus", "networkEnable", "networkFlush"],
    advanced: [],
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
