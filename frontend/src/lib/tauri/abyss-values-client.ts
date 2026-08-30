import {
  abyssValuesError,
  parseAbyssPredictionTeams,
  parseAbyssValuesSnapshot,
  type AbyssHalfId,
  type AbyssPredictionTeams,
  type AbyssValuesSnapshot,
} from "./abyss-values-contract";
import { TechnicalContractError } from "./technical-contract";
import { tauriInvokeTransport, type InvokeTransport } from "./stream-client";

const COMMANDS = {
  clearTeam: "clear_abyss_prediction_team",
  getSnapshot: "get_abyss_values_snapshot",
  importTeam: "import_abyss_prediction_team",
  importCurrentTeam: "import_current_abyss_prediction_team",
  swapTeams: "swap_abyss_prediction_teams",
} as const;

export interface AbyssValuesClient {
  getSnapshot(): Promise<AbyssValuesSnapshot>;
  importTeam(half: AbyssHalfId, json: string): Promise<AbyssPredictionTeams>;
  importCurrentTeam(half: AbyssHalfId): Promise<AbyssPredictionTeams>;
  clearTeam(half: AbyssHalfId): Promise<AbyssPredictionTeams>;
  swapTeams(): Promise<AbyssPredictionTeams>;
}

export function createAbyssValuesClient(
  transport: InvokeTransport = tauriInvokeTransport,
): AbyssValuesClient {
  async function command<T>(
    name: string,
    parser: (value: unknown) => T,
    arguments_?: Record<string, unknown>,
  ): Promise<T> {
    try {
      return parser(await transport.invoke(name, arguments_));
    } catch (error) {
      if (error instanceof TechnicalContractError) {
        throw error;
      }
      throw abyssValuesError(error);
    }
  }

  return {
    getSnapshot: () => command(COMMANDS.getSnapshot, parseAbyssValuesSnapshot),
    importTeam: (half, json) =>
      command(COMMANDS.importTeam, parseAbyssPredictionTeams, {
        half,
        json,
      }),
    importCurrentTeam: (half) =>
      command(COMMANDS.importCurrentTeam, parseAbyssPredictionTeams, { half }),
    clearTeam: (half) =>
      command(COMMANDS.clearTeam, parseAbyssPredictionTeams, { half }),
    swapTeams: () => command(COMMANDS.swapTeams, parseAbyssPredictionTeams),
  };
}

export const abyssValuesClient = createAbyssValuesClient();
