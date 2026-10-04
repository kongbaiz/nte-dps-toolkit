import { describe, expect, it } from "vitest";

import { createDamageImageLookup } from "./damage-image-lookup";

describe("damage image lookup", () => {
  it("indexes damage digits by exact file stem", () => {
    const lookup = createDamageImageLookup(
      {
        "/res/images/font/tiaozi1/ling_0.png": "ling-zero",
        "/res/images/font/tiaozi1/not_ling_0.png": "other-zero",
        "/res/images/font/tiaozi1/ling_1.png": "ling-one",
      },
      {},
    );

    expect(lookup.digit("ling", "0")).toBe("ling-zero");
    expect(lookup.digit("ling", "1")).toBe("ling-one");
    expect(lookup.digit("ling", "2")).toBeUndefined();
  });

  it("loads all ten dedicated malice digits without substituting elemental assets", () => {
    const images = import.meta.glob<string>(
      "@res/images/font/tiaozi1/heiyu_*.png",
      {
        eager: true,
        query: "?url&no-inline",
        import: "default",
      },
    );
    const lookup = createDamageImageLookup(images, {});
    expect(Object.keys(images)).toHaveLength(10);
    for (let digit = 0; digit <= 9; digit++) {
      expect(lookup.digit("heiyu", String(digit))).toContain(
        `heiyu_${digit}.png`,
      );
      expect(lookup.digit("hun", String(digit))).toBeUndefined();
    }
  });
  it("returns localized reaction parts in display order", () => {
    const lookup = createDamageImageLookup(
      {},
      {
        "/res/images/font/tiaozi1/zh/fanying03_02.png": "zh-part-two",
        "/res/images/font/tiaozi1/en/fanying03_01.png": "en-part-one",
        "/res/images/font/tiaozi1/zh/fanying03_01.png": "zh-part-one",
      },
    );

    expect(lookup.reaction("zh", 3)).toEqual(["zh-part-one", "zh-part-two"]);
    expect(lookup.reaction("en", 3)).toEqual(["en-part-one"]);
    expect(lookup.reaction("ja", 3)).toEqual([]);
  });

  it("enumerates module records only while constructing the lookup", () => {
    let digitEnumerations = 0;
    let reactionEnumerations = 0;
    const digitImages = countEnumerations(
      { "/res/images/font/tiaozi1/ling_7.png": "ling-seven" },
      () => {
        digitEnumerations += 1;
      },
    );
    const reactionImages = countEnumerations(
      {
        "/res/images/font/tiaozi1/ja/fanying12_01.png": "ja-part-one",
      },
      () => {
        reactionEnumerations += 1;
      },
    );

    const lookup = createDamageImageLookup(digitImages, reactionImages);
    expect(digitEnumerations).toBe(1);
    expect(reactionEnumerations).toBe(1);

    expect(lookup.digit("ling", "7")).toBe("ling-seven");
    expect(lookup.digit("ling", "7")).toBe("ling-seven");
    expect(lookup.reaction("ja", 12)).toEqual(["ja-part-one"]);
    expect(lookup.reaction("ja", 12)).toEqual(["ja-part-one"]);
    expect(digitEnumerations).toBe(1);
    expect(reactionEnumerations).toBe(1);
  });
});

function countEnumerations(
  images: Record<string, string>,
  onEnumerate: () => void,
): Record<string, string> {
  return new Proxy(images, {
    ownKeys(target) {
      onEnumerate();
      return Reflect.ownKeys(target);
    },
  });
}
