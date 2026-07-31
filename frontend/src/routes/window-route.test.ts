import { describe, expect, it } from "vitest";

import { resolveWindowRoute } from "./window-route";

describe("window route registry", () => {
  it("registers the stable HUD, Console, and abyss-value slots", () => {
    expect(resolveWindowRoute("hud-spike")).toBe("technical-hud");
    expect(resolveWindowRoute("console")).toBe("console");
    expect(resolveWindowRoute("abyss-values")).toBe("abyss-values");
  });

  it("rejects anonymous windows", () => {
    expect(resolveWindowRoute("anonymous")).toBe("unsupported");
    expect(resolveWindowRoute("toString")).toBe("unsupported");
  });
});
