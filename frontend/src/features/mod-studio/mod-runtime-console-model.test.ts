import { describe, expect, it } from "vitest";

import type { ModStudioRuntimeEntry } from "@/lib/tauri/mod-studio-contract";

import {
  runtimeEntryText,
  visibleRuntimeEntries,
} from "./mod-runtime-console-model";

const entries: ModStudioRuntimeEntry[] = [
  {
    kind: "log",
    sequence: "9",
    nativeSequence: "4",
    timestamp100ns: "133000000000000000",
    modId: "runtime",
    level: "warning",
    message: "Compilation failed; previous version kept.",
    messageKey: "Compilation failed; previous version kept.",
    messageArguments: [],
  },
  {
    kind: "event",
    sequence: "10",
    nativeSequence: "7",
    timestamp100ns: "133000000000000010",
    modId: "telemetry",
    name: "post.sample",
    values: ["15"],
  },
];

describe("Mod runtime console model", () => {
  it("filters by source and keeps new entries after a local clear", () => {
    expect(
      visibleRuntimeEntries(
        entries,
        "2",
        { generation: "2", throughSequence: "9" },
        "event",
      ),
    ).toEqual([entries[1]]);
    expect(
      visibleRuntimeEntries(
        entries,
        "3",
        { generation: "2", throughSequence: "10" },
        "warning",
      ),
    ).toEqual([entries[0]]);
  });

  it("keeps raw logs and formats event values as decimal and hexadecimal", () => {
    expect(runtimeEntryText(entries[0], (key) => `translated:${key}`)).toBe(
      "translated:Compilation failed; previous version kept.",
    );
    expect(runtimeEntryText(entries[1], (key) => key)).toBe(
      "post.sample v0=15/0x000000000000000F",
    );
  });
});
