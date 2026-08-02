import { describe, expect, it } from "vitest";

import { abyssValuesFixture } from "./abyss-values-contract.test";
import { createAbyssValuesClient } from "./abyss-values-client";

describe("abyss values client", () => {
  it("uses stable commands and explicit half arguments", async () => {
    const calls: Array<{
      command: string;
      arguments_?: Record<string, unknown>;
    }> = [];
    const client = createAbyssValuesClient({
      invoke: async (command, arguments_) => {
        calls.push({ command, arguments_ });
        if (command === "get_abyss_values_snapshot") {
          return abyssValuesFixture();
        }
        return {
          upper: null,
          lower: null,
        };
      },
    });

    await client.getSnapshot();
    await client.importTeam("upper", '{"version":1}');
    await client.importCurrentTeam("lower");
    await client.clearTeam("lower");
    await client.swapTeams();

    expect(calls).toEqual([
      { command: "get_abyss_values_snapshot", arguments_: undefined },
      {
        command: "import_abyss_prediction_team",
        arguments_: { half: "upper", json: '{"version":1}' },
      },
      {
        command: "import_current_abyss_prediction_team",
        arguments_: { half: "lower" },
      },
      {
        command: "clear_abyss_prediction_team",
        arguments_: { half: "lower" },
      },
      { command: "swap_abyss_prediction_teams", arguments_: undefined },
    ]);
  });
});
