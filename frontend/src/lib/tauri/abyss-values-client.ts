import { invokeCommand } from "@/lib/tauri/command-error";
import type { AbyssHalf } from "@/lib/tauri/generated/AbyssHalf";
import type { AbyssPredictionTeamsSnapshot } from "@/lib/tauri/generated/AbyssPredictionTeamsSnapshot";
import type { AbyssValuesSnapshot } from "@/lib/tauri/generated/AbyssValuesSnapshot";

const teams = (command: string, arguments_?: Record<string, unknown>) =>
  invokeCommand<AbyssPredictionTeamsSnapshot>(command, arguments_);

export const abyssValuesClient = {
  getSnapshot: () =>
    invokeCommand<AbyssValuesSnapshot>("get_abyss_values_snapshot"),
  importTeam: (half: AbyssHalf, json: string) =>
    teams("import_abyss_prediction_team", { half, json }),
  importCurrentTeam: (half: AbyssHalf) =>
    teams("import_current_abyss_prediction_team", { half }),
  clearTeam: (half: AbyssHalf) =>
    teams("clear_abyss_prediction_team", { half }),
  swapTeams: () => teams("swap_abyss_prediction_teams"),
};
