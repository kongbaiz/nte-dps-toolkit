import { describe, expect, it } from "vitest";
import art from "@res/data/characters/panel_art.json";
import {
  formatPanelNumber,
  formatModifier,
  panelImage,
} from "./character-build-model";
describe("character build presentation", () => {
  it("resolves live FName casing for 焰魂狂飙 without replacing its actual artwork", () => {
    const expected = panelImage("arc", "fork_wushoutieyu");
    expect(expected).toContain("fork_wushoutieyu_256");
    expect(panelImage("arc", "fork_Wushoutieyu")).toBe(expected);
    expect(panelImage("arc", "FORK_WUSHOUTIEYU")).toBe(expected);
    expect(panelImage("arc", "fork_wushoutieyu_unknown")).toBeNull();
  });
  it("keeps exported arc FName keys unique and every icon resolvable", () => {
    const ids = Object.keys(art.arcs);
    expect(new Set(ids.map((id) => id.toLowerCase())).size).toBe(ids.length);
    for (const [id, path] of Object.entries(art.arcs)) {
      if (path !== null)
        expect(panelImage("arc", id.toUpperCase())).not.toBeNull();
    }
  });
  it("matches the reference precision without fixed zero padding", () => {
    expect(formatPanelNumber("21114.49", "integer")).toBe("21114");
    expect(formatPanelNumber("2122.345", "attack")).toBe("2122.3");
    expect(formatPanelNumber("884", "integer")).toBe("884");
    expect(formatPanelNumber("0.91", "percent")).toBe("91%");
    expect(formatPanelNumber("1.78", "percent")).toBe("178%");
    expect(formatPanelNumber("350", "score")).toBe("350.0");
    expect(formatModifier("AtkUp", "0.125")).toBe("+12.5%");
    expect(formatModifier("AtkUp", "0.0375")).toBe("+3.75%");
    expect(formatModifier("CritDamageBase", "0.6")).toBe("+60%");
    expect(formatModifier("DamageUpGeneralBase", "0.02")).toBe("+2%");
  });
  it("keeps null, zero, unknown units and unsafe integers distinct", () => {
    expect(formatModifier("CritBase", null)).toBe("—");
    expect(formatModifier("CritBase", "0")).toBe("0%");
    expect(formatModifier("CritBase", "-0.0000001")).toBe("0%");
    expect(panelImage("equipment", "__proto__")).toBeNull();
    expect(formatModifier("new_unknown_attribute", "0.123456789")).toBe("0.12");
    expect(formatPanelNumber("18446744073709551615")).toBe(
      "18446744073709551615",
    );
    expect(formatPanelNumber("fixture_id")).toBe("fixture_id");
  });
  it("resolves exact character, arc, cassette and drive IDs to bundled images", () => {
    expect(panelImage("avatar", "1004")).toContain("player_004");
    expect(panelImage("avatar", "not_a_real_id")).toBeNull();
    expect(panelImage("character", "1042")).toContain("heiyu");
    expect(panelImage("arc", "fork_twinbirds")).toContain("fork_twinbirds_256");
    expect(panelImage("equipment", "Psyche_orange")).toContain("kongmu");
    expect(panelImage("equipment", "cell2_style1_1_Orange")).toContain(
      "Equip41",
    );
    expect(panelImage("arc", "not_a_real_id")).toBeNull();
    expect(panelImage("equipment", null)).toBeNull();
  });
});
