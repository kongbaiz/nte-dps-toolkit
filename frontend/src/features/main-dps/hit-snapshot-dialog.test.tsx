import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, expect, it } from "vitest";
import { setFrontendLanguage } from "@/lib/i18n";
import {
  AttributeSnapshotView,
  EffectSnapshotView,
} from "./hit-snapshot-dialog";
import { hitSnapshotFixture } from "../../../dev/hit-snapshot-fixture";
beforeEach(() => setFrontendLanguage("en"));
it("renders frozen attributes including real zero and explicit unknown", () => {
  const html = renderToStaticMarkup(
    <AttributeSnapshotView
      snapshot={hitSnapshotFixture.data!.attackerAttributes}
    />,
  );
  expect(html).toContain(">12345.50<");
  expect(html).toContain(">0.00<");
  expect(html).toContain("Unknown");
  expect(html).toContain("HTAttributeComponent.GetCrit");
});
it("bounds the first effect page and exposes a way to read the rest", () => {
  const html = renderToStaticMarkup(
    <EffectSnapshotView snapshot={hitSnapshotFixture.data!.attackerEffects} />,
  );
  expect(html).toContain("Load more");
  expect(html).toContain("50");
  expect(html).not.toContain("示例状态 60");
  const partial = renderToStaticMarkup(
    <EffectSnapshotView snapshot={hitSnapshotFixture.data!.victimEffects} />,
  );
  expect(partial).toContain("Partial snapshot");
});

it.each([
  "hp",
  "maxHp",
  "shield",
  "attack",
  "defense",
  "chargeCurrent",
  "chargeMax",
  "unbalCurrent",
  "unbalMax",
  "unbalSpeed",
  "unbalIntensity",
  "unbalReduceNatur",
  "unbalValueAdd",
])(
  "formats %s with two decimal places without changing the snapshot",
  (key) => {
    const snapshot = {
      ...hitSnapshotFixture.data!.attackerAttributes!,
      values: { [key]: { value: 1234.567, status: "ok", source: "fixture" } },
    };
    const before = JSON.stringify(snapshot);
    const html = renderToStaticMarkup(
      <AttributeSnapshotView snapshot={snapshot} />,
    );
    expect(html).toContain(">1234.57<");
    expect(JSON.stringify(snapshot)).toBe(before);
  },
);
it.each([
  "crit",
  "critDamage",
  "resistNormal",
  "resistCosmos",
  "resistNature",
  "resistIncantation",
  "resistChaos",
  "resistPsyche",
  "resistLakshana",
  "resistPsychically",
  "damageUpGeneral",
  "damageUpNormal",
  "damageUpCosmos",
  "damageUpNature",
  "damageUpIncantation",
  "damageUpChaos",
  "damageUpPsyche",
  "damageUpLakshana",
  "damageUpPsychically",
  "unbalAccrueEfficiency",
  "unbalAntiAccrueEfficiency",
  "unbalBonus",
])(
  "formats %s as a two-decimal percentage including zero, negatives and values above one",
  (key) => {
    for (const [value, expected] of [
      [0, "0.00%"],
      [0.63126, "63.13%"],
      [1.5, "150.00%"],
      [-0.125, "-12.50%"],
    ] as const) {
      const snapshot = {
        ...hitSnapshotFixture.data!.attackerAttributes!,
        values: { [key]: { value, status: "ok", source: "fixture" } },
      };
      const html = renderToStaticMarkup(
        <AttributeSnapshotView snapshot={snapshot} />,
      );
      expect(html).toContain(`>${expected}<`);
      expect(html).not.toContain("(raw)");
      expect(snapshot.values[key].value).toBe(value);
    }
  },
);
it("preserves unknown values and booleans and does not guess units for future fields", () => {
  const snapshot = {
    ...hitSnapshotFixture.data!.attackerAttributes!,
    values: {
      crit: { value: null, status: "call_failed", source: "fixture" },
      isBalancedingState: { value: false, status: "ok", source: "fixture" },
      futureBonus: { value: 0.12345, status: "ok", source: "fixture" },
    },
  };
  const html = renderToStaticMarkup(
    <AttributeSnapshotView snapshot={snapshot} />,
  );
  expect(html).toContain(">Unknown value<");
  expect(html).toContain(">No<");
  expect(html).toContain(">0.12345<");
});
it.each([
  [0, "0.00"],
  [12.5, "12.50"],
  [12.567, "12.57"],
  [-1, "Permanent"],
] as const)(
  "formats duration %s while preserving permanent effects and integer stacks",
  (duration, expected) => {
    const base = hitSnapshotFixture.data!.attackerEffects!;
    const snapshot = {
      ...base,
      effects: [
        { ...base.effects![0], duration, stacks: 3, startWorldTime: 15.678 },
      ],
    };
    const before = JSON.stringify(snapshot);
    const html = renderToStaticMarkup(
      <EffectSnapshotView snapshot={snapshot} />,
    );
    expect(html).toContain(`>${expected}<`);
    expect(html).toContain(">3<");
    expect(html).toContain("15.68");
    expect(JSON.stringify(snapshot)).toBe(before);
  },
);
