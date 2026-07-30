import { describe, expect, it } from "vitest";

import { resolveWindowRoute } from "./window-route";

describe("window route registry", () => {
  it("registers the technical HUD slot and rejects anonymous windows", () => {
    expect(resolveWindowRoute("hud-spike")).toBe("technical-hud");
    expect(resolveWindowRoute("anonymous")).toBe("unsupported");
    expect(resolveWindowRoute("toString")).toBe("unsupported");
  });
});
