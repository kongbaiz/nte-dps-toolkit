import { describe, expect, it } from "vitest";

import {
  formatByteCount,
  formatUpdateByteProgress,
  updateComponentLabelKey,
  updateProgressPercent,
} from "./update-presentation";

describe("update presentation", () => {
  it.each([
    ["0", "0 B"],
    ["1023", "1023 B"],
    ["1024", "1.0 KiB"],
    ["1536", "1.5 KiB"],
    ["1048575", "1023.9 KiB"],
    ["1048576", "1.0 MiB"],
    ["1073741824", "1.0 GiB"],
  ])("formats %s bytes with binary unit semantics", (value, expected) => {
    expect(formatByteCount(value)).toBe(expected);
  });

  it("keeps invalid byte counts visible instead of inventing a size", () => {
    expect(formatByteCount("-1")).toBe("-1");
    expect(formatByteCount("not-a-number")).toBe("not-a-number");
  });

  it("bounds known progress and represents an unknown total explicitly", () => {
    expect(updateProgressPercent("0", "100")).toBe(0);
    expect(updateProgressPercent("1", "3")).toBe(33);
    expect(updateProgressPercent("3", "3")).toBe(100);
    expect(updateProgressPercent("4", "3")).toBe(100);
    expect(updateProgressPercent("0", "0")).toBeNull();
    expect(updateProgressPercent("invalid", "10")).toBeNull();
  });

  it("omits the misleading zero total when download size is unknown", () => {
    expect(formatUpdateByteProgress("0", "0")).toBe("0 B");
    expect(formatUpdateByteProgress("1024", "2048")).toBe("1.0 KiB / 2.0 KiB");
  });

  it("uses the shared component label keys", () => {
    expect(updateComponentLabelKey("app")).toBe("Application");
    expect(updateComponentLabelKey("mods-plugin")).toBe("Mod loader");
  });
});
