import { afterEach, describe, expect, it, vi } from "vitest";

import { downloadTextFile } from "./download-text-file";

describe("downloadTextFile", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("keeps the object URL alive until after the mounted link is clicked", () => {
    vi.useFakeTimers();
    const click = vi.fn();
    const remove = vi.fn();
    const append = vi.fn();
    const link = { click, download: "", hidden: false, href: "", remove };
    const createObjectURL = vi.fn(() => "blob:team-data");
    const revokeObjectURL = vi.fn();

    vi.stubGlobal("document", {
      body: { append },
      createElement: vi.fn(() => link),
    });
    vi.stubGlobal("URL", { createObjectURL, revokeObjectURL });

    downloadTextFile("nte-team-dps.json", '{"version":1}');

    expect(append).toHaveBeenCalledWith(link);
    expect(link.download).toBe("nte-team-dps.json");
    expect(link.href).toBe("blob:team-data");
    expect(click).toHaveBeenCalledOnce();
    expect(remove).toHaveBeenCalledOnce();
    expect(revokeObjectURL).not.toHaveBeenCalled();

    vi.runAllTimers();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:team-data");
  });
});
