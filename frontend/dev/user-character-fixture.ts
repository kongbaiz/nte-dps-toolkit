// Synthetic UI data only. Never imported by production entries.
import type { UserCharacters } from "@/lib/tauri/user-characters-client";
export function userCharacterFixture(): UserCharacters {
  const summary = {
    uid: "1:2",
    itemId: "fixture_character",
    name: "演示角色（非实机）",
    level: "60",
  };
  const entry = (
    fields: [string, string | null][],
    name: string | null = null,
  ) => ({ name, fields: fields.map(([label, value]) => ({ label, value })) });
  return {
    contractVersion: 1,
    connectionIdentity: "42:99",
    state: "completed",
    dirty: false,
    sdkCompatible: true,
    page: {
      snapshotId: "fixture:1",
      observedUnixUs: "1790000000000000",
      complete: true,
      total: 1,
      offset: 0,
      records: [summary],
      detail: {
        summary,
        sections: [
          {
            title: "Character progression",
            available: true,
            entries: [
              entry([
                ["Character level", "60"],
                ["Breakthrough level", "4"],
                ["Awakening level", "2"],
                ["Experience", "0"],
              ]),
            ],
          },
          {
            title: "Arc",
            available: true,
            entries: [
              entry(
                [
                  ["Item ID", "fixture_arc"],
                  ["Enhancement level", "50"],
                  ["Breakthrough level", "3"],
                  ["Star level", "1"],
                ],
                "演示弧盘",
              ),
            ],
          },
          {
            title: "Skill levels",
            available: true,
            entries: [
              entry([
                ["Skill ID", "fixture_skill"],
                ["Saved level", "7"],
                ["Base level", "7"],
                ["Awakening bonus", "2"],
                ["Effective level", "9"],
              ]),
            ],
          },
          {
            title: "Active awakenings",
            available: true,
            entries: [
              entry([
                ["Awakening slot", "2"],
                ["Awakening number", "5"],
                ["Effect ID", "fixture_effect_99"],
                ["Effect name", "演示觉醒效果"],
              ]),
            ],
          },
          { title: "Cassettes", available: true, entries: [] },
          {
            title: "Drive blocks",
            available: true,
            entries: [
              entry(
                [
                  ["Item ID", "fixture_drive"],
                  ["Enhancement level", "12"],
                  ["Attribute stage", "inventory_raw"],
                  ["Attribute units", "unknown"],
                  ["fixture_attack", "0"],
                  ["fixture_rate", "0.123456789"],
                ],
                "演示驱动块",
              ),
            ],
          },
          { title: "Runtime attributes", available: false, entries: [] },
        ],
      },
    },
  };
}

// Artwork/format QA only: identifiers select real bundled images; every value is synthetic.
export function userCharacterArtworkFixture(): UserCharacters {
  const fixture = userCharacterFixture();
  const page = fixture.page!;
  page.records[0] = {
    uid: "1:2",
    itemId: "1042",
    name: "黑羽（演示）",
    level: "80",
  };
  page.detail!.summary = page.records[0];
  const sections = page.detail!.sections;
  sections[1].entries[0].name = "罪与罚（演示）";
  sections[1].entries[0].fields.find((f) => f.label === "Item ID")!.value =
    "fork_twinbirds";
  const baseSkill = sections[2].entries[0];
  sections[2].entries = [
    "Normal attack",
    "Variation skill",
    "Ultimate finale",
    "Assist skill",
  ].map((name) => ({
    ...baseSkill,
    name,
    fields: baseSkill.fields.map((f) => ({
      ...f,
      value: f.label === "Skill ID" ? name : f.value,
    })),
  }));
  // Synthetic account values for visual QA; never read by production entries.
  sections[6] = {
    title: "Runtime attributes",
    available: true,
    entries: [
      {
        name: "Account configuration attributes",
        fields: [
          { label: "HPMaxBase", value: "20750" },
          { label: "AtkBase", value: "2371.1" },
          { label: "DefBase", value: "981" },
          { label: "CritBase", value: "0.57" },
          { label: "CritDamageBase", value: "1.06" },
          { label: "DamageUpGeneralBase", value: "0.02" },
          { label: "ChargeGetEfficiencyBase", value: "1" },
        ],
      },
    ],
  };
  const entry = (
    id: string,
    name: string | null,
    stats: [string, string][],
  ) => ({
    name,
    fields: [
      { label: "Item ID", value: id },
      { label: "Enhancement level", value: "20" },
      { label: "Attribute units", value: "unknown" },
      ...stats.map(([label, value]) => ({ label, value })),
    ],
  });
  sections[4].entries = [
    entry("Psyche_orange", "恶魔之血（演示）", [
      ["CritDamageBase", "0.6"],
      ["AtkUp", "0.125"],
      ["CritBase", "0.1"],
      ["DamageUpGeneralBase", "0.1"],
      ["CritDamageAdd", "0.2"],
    ]),
  ];
  sections[5].entries = [
    "cell2_style1_1_Orange",
    "cell2_style2_1_Orange",
    "cell4_style6_1_Orange",
    "cell3_style1_1_Orange",
    "cell3_style2_1_Orange",
    "cell3_style4_1_Orange",
    "cell3_style6_1_Orange",
  ].map((id) =>
    entry(id, null, [
      ["CritDamageBase", "0.06"],
      ["CritBase", "0.03"],
      ["DamageUpGeneralBase", "0.03"],
      ["AtkUp", "0.0375"],
    ]),
  );
  return fixture;
}
