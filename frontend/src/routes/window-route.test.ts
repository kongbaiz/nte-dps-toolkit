import { describe, expect, it } from "vitest";

import { resolveWindowRoute } from "./window-route";

describe("window route registry", () => {
  it("registers the stable HUD and Console slots", () => {
    expect(resolveWindowRoute("hud-spike")).toBe("technical-hud");
    expect(resolveWindowRoute("console")).toBe("mod-studio");
  });

  it("rejects anonymous windows", () => {
    expect(resolveWindowRoute("anonymous")).toBe("unsupported");
    expect(resolveWindowRoute("toString")).toBe("unsupported");
  });
});
