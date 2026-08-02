import { describe, expect, it } from "vitest";

import { createConsoleControlClient } from "./console-control-client";

describe("Console control client", () => {
  it("uses the typed Console command and camelCase action argument", async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = [];
    const client = createConsoleControlClient({
      invoke: async (command, arguments_) => {
        calls.push([command, arguments_]);
      },
    });

    await client.execute("toggle-capture");

    expect(calls).toEqual([
      ["execute_console_control", { action: "toggle-capture" }],
    ]);
  });
});
