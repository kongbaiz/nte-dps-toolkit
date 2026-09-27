import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  userCharacterFixture,
  userCharacterArtworkFixture,
} from "../../../dev/user-character-fixture";
import { setFrontendLanguage } from "@/lib/i18n";
import {
  CharacterDataPage,
  CharacterSnapshotDetail,
} from "./character-data-page";
const model = vi.hoisted(() => ({
  snapshot: null as unknown,
  pending: false,
  error: null as string | null,
  load: vi.fn(),
}));
vi.mock("./use-user-characters", () => ({ useUserCharacters: () => model }));

describe("read-only character page", () => {
  beforeEach(() => {
    setFrontendLanguage("en");
    model.snapshot = userCharacterFixture();
    model.error = null;
    model.pending = false;
  });
  it("replaces the resource editor with plugin snapshot controls and all sections", () => {
    const html = renderToStaticMarkup(<CharacterDataPage />);
    for (const label of [
      "Refresh from game",
      "Check snapshot",
      "Skill levels",
      "Active awakenings",
      "Cassettes",
      "Drive blocks",
      "Runtime attributes",
    ])
      expect(html).toContain(label);
    for (const label of [
      "Save to characters.json",
      "New ID",
      "Edit Character",
      "Avatar Path",
      "Chinese Name",
      "Unsaved changes",
    ])
      expect(html).not.toContain(label);
    expect(html).toContain("not an atomic snapshot");
    expect(html).toContain("unknown modifiers retain raw units");
    expect(html).toContain("0.123456789");
  });
  it("distinguishes unknown from unequipped or zero", () => {
    const detail = userCharacterFixture().page!.detail!;
    detail.sections[1].available = false;
    detail.sections[1].entries = [];
    const html = renderToStaticMarkup(
      <CharacterSnapshotDetail detail={detail} />,
    );
    expect(html).toContain("Not fully observed");
    expect(html).toContain("None observed");
    expect(html).toContain(">0<");
    expect(html).toContain("Live attributes unavailable");
  });
  it("uses catalog portraits in the sidebar and actual awakening numbers, never slot numbers", () => {
    const fixture = userCharacterArtworkFixture();
    model.snapshot = fixture;
    const html = renderToStaticMarkup(<CharacterDataPage />);
    expect(html).toContain("player_heiyu");
    expect(html).toContain("<b>5</b>");
    expect(html).not.toContain("<b>2</b>");
    const awakening = fixture.page!.detail!.sections[3].entries[0];
    awakening.fields = awakening.fields.filter(
      (field) => field.label !== "Awakening number",
    );
    expect(renderToStaticMarkup(<CharacterDataPage />)).toContain("<b>—</b>");
  });
  it("labels all-account attributes separately from team combat buffs", () => {
    const detail = userCharacterFixture().page!.detail!;
    detail.sections[6] = {
      title: "Runtime attributes",
      available: true,
      entries: [
        {
          name: "Account configuration attributes",
          fields: [
            { label: "HPMaxBase", value: "20750" },
            { label: "ChargeGetEfficiencyBase", value: "1" },
          ],
        },
      ],
    };
    const html = renderToStaticMarkup(
      <CharacterSnapshotDetail detail={detail} />,
    );
    expect(html).toContain(">20750<");
    expect(html).toContain(">100%<");
    expect(html).toContain("Account attributes computed by the game");
    expect(html).not.toContain("current buffs are included");
  });
  it("does not translate raw values using unrelated attribution or session labels", () => {
    setFrontendLanguage("zh-CN");
    const html = renderToStaticMarkup(
      <CharacterSnapshotDetail detail={userCharacterFixture().page!.detail!} />,
    );
    expect(html).toContain("原始值");
    expect(html).not.toContain("未知归因");
    expect(html).not.toContain("当前会话");
  });
  it("shows partial snapshots and connection failures without editable fallbacks", () => {
    const partial = userCharacterFixture();
    partial.page!.complete = false;
    model.snapshot = partial;
    expect(renderToStaticMarkup(<CharacterDataPage />)).toContain(
      "Partial snapshot",
    );
    model.snapshot = null;
    model.error =
      "Select Plugin mode on the home page to read character snapshots.";
    const html = renderToStaticMarkup(<CharacterDataPage />);
    expect(html).toContain("Select Plugin mode");
    expect(html).not.toContain("fixture_character");
  });
  it("renders real mapped artwork and reference-style decimals without mutating the snapshot", () => {
    const fixture = userCharacterArtworkFixture();
    const before = JSON.stringify(fixture);
    const html = renderToStaticMarkup(
      <CharacterSnapshotDetail detail={fixture.page!.detail!} />,
    );
    expect(html).toContain("fork_twinbirds_256");
    expect(html).toContain("Equip41");
    expect(html).toContain("kongmu");
    expect(html).toContain(">+60%<");
    expect(html).toContain(">+12.5%<");
    expect(html).toContain(">+3.75%<");
    expect(html).not.toContain("+60.00%");
    expect(JSON.stringify(fixture)).toBe(before);
  });
  it("keeps the overview focused while retaining UID and raw evidence in disclosures", () => {
    const html = renderToStaticMarkup(
      <CharacterSnapshotDetail
        detail={userCharacterArtworkFixture().page!.detail!}
      />,
    );
    const details = html.match(/<details\b[^>]*>[\s\S]*?<\/details>/g)!;
    expect(
      details.some(
        (section) =>
          section.includes("Character UID") && section.includes("1:2"),
      ),
    ).toBe(true);
    expect(
      details.every((section) => !/^<details[^>]*\bopen/.test(section)),
    ).toBe(true);
    expect(html).not.toContain("Equipment snapshot");
    for (const value of [
      ">20750<",
      ">2371.1<",
      ">981<",
      ">57%<",
      ">106%<",
      ">2%<",
      ">100%<",
    ])
      expect(html).toContain(value);
    expect(html).toContain('title="0.57"');
  });
  it("keeps pagination names accessible without squeezing the count", () => {
    const html = renderToStaticMarkup(<CharacterDataPage />);
    expect(html).toContain('aria-label="Previous"');
    expect(html).toContain('aria-label="Next"');
    expect(html).toContain("Total 1 entries");
  });
  it("shows semantic skill labels and live runtime values, not raw ability IDs", () => {
    setFrontendLanguage("zh-CN");
    const fixture = userCharacterFixture();
    const detail = fixture.page!.detail!;
    const original = detail.sections[2].entries[0];
    detail.sections[2].entries = [
      "Normal attack",
      "Variation skill",
      "Ultimate finale",
      "Assist skill",
    ].map((name) => ({ ...original, name }));
    detail.sections[6] = {
      title: "Runtime attributes",
      available: true,
      entries: [
        {
          name: "Live team actor snapshot",
          fields: [
            { label: "HPMaxBase", value: "21114" },
            { label: "AtkBase", value: "2122.345" },
            { label: "CritBase", value: "0.91" },
          ],
        },
      ],
    };
    const html = renderToStaticMarkup(
      <CharacterSnapshotDetail detail={detail} />,
    );
    for (const text of [
      "普通攻击",
      "变轨技能",
      "极轨终结",
      "援护技",
      ">21114<",
      ">2122.3<",
      ">91%<",
    ])
      expect(html).toContain(text);
    expect(html).not.toContain("实时属性不可用");
    expect(html).not.toContain("<span>fixture_skill</span>");
  });
});
