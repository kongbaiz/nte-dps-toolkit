import { describe, expect, it } from "vitest";

import {
  parseEmptyCurtainEvent,
  parseEmptyCurtainSnapshot,
} from "@/lib/tauri/empty-curtain-contract";

export const EMPTY_CURTAIN_FIXTURE = {
  contractVersion: 2,
  generation: "9007199254740992",
  observedAtUnixMs: "1785542400000",
  hasData: true,
  complete: true,
  characters: [
    { uid: { slot: 1, serial: 2 }, characterId: 1076, name: "Character" },
  ],
  items: [
    {
      uid: { slot: 3, serial: 4 },
      itemId: "cell4_style1_1_Orange",
      filterId: "cell4_style1_1",
      kind: "module",
      quality: "orange",
      name: "Module",
      icon: "module.png",
      level: 20,
      maxLevel: 20,
      locked: false,
      discarded: false,
      equippedCharacterUid: { slot: 1, serial: 2 },
      equippedCharacterId: 1076,
      equippedPlacement: { row: 0, column: 1 },
      stats: [
        {
          property: "AtkAdd",
          label: "Attack",
          value: 12,
          percent: false,
          main: true,
          unlockLevel: null,
          unlocked: true,
        },
      ],
      setName: null,
      setEffects: [],
    },
  ],
  operation: {
    status: "idle",
    messageKey: "No equipment operation is pending",
    messageArguments: [],
  },
};

describe("Console equipment contract", () => {
  it("keeps large generations as decimal strings and parses stable item uids", () => {
    const snapshot = parseEmptyCurtainSnapshot(EMPTY_CURTAIN_FIXTURE);
    expect(snapshot.generation).toBe("9007199254740992");
    expect(snapshot.items[0].uid).toEqual({ slot: 3, serial: 4 });
    expect(snapshot.items[0].stats[0]).toMatchObject({
      unlockLevel: null,
      unlocked: true,
    });
  });

  it("preserves secondary-stat unlock requirements", () => {
    const snapshot = parseEmptyCurtainSnapshot({
      ...EMPTY_CURTAIN_FIXTURE,
      items: [
        {
          ...EMPTY_CURTAIN_FIXTURE.items[0],
          stats: [
            {
              property: "CritAdd",
              label: "Critical Rate",
              value: 0.08,
              percent: true,
              main: false,
              unlockLevel: 15,
              unlocked: false,
            },
          ],
        },
      ],
    });

    expect(snapshot.items[0].stats[0]).toMatchObject({
      unlockLevel: 15,
      unlocked: false,
    });
  });

  it("accepts tagged snapshots and rejects unbounded item details", () => {
    expect(
      parseEmptyCurtainEvent({
        event: "snapshot",
        payload: EMPTY_CURTAIN_FIXTURE,
      }).items,
    ).toHaveLength(1);
    expect(() =>
      parseEmptyCurtainSnapshot({
        ...EMPTY_CURTAIN_FIXTURE,
        items: [
          {
            ...EMPTY_CURTAIN_FIXTURE.items[0],
            stats: Array.from(
              { length: 17 },
              () => EMPTY_CURTAIN_FIXTURE.items[0].stats[0],
            ),
          },
        ],
      }),
    ).toThrow(/detail bounds/);
  });
});
