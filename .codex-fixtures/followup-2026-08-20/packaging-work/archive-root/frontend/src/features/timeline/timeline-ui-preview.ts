import type { TimelinePreviewSnapshot } from "./timeline-ui-model";

const TEAM_DPS = [
  0, 18500, 42800, 76200, 101500, 124000, 139500, 127000, 98000, 111000, 132500,
  158000, 176000, 149000, 121500, 96500, 82000, 104000, 137500, 163000, 151000,
  119000, 94000, 72500,
];

const CHARACTER_DPS = [
  [
    0, 11800, 29100, 50100, 69000, 80500, 91700, 80600, 61200, 73100, 86400,
    99300, 110200, 91500, 72400, 58100, 47600, 65100, 88200, 101400, 94100,
    73500, 56200, 43100,
  ],
  [
    0, 4100, 9200, 16800, 21400, 27300, 30200, 29400, 22600, 25100, 31700,
    38600, 41500, 36700, 30200, 24100, 20800, 25100, 32100, 40100, 35800, 28600,
    23100, 17200,
  ],
  [
    0, 2600, 4500, 9300, 11100, 14500, 15900, 17000, 14200, 12800, 14400, 18300,
    21300, 18100, 15400, 11800, 10100, 12600, 17200, 19900, 18100, 13900, 10800,
    7900,
  ],
] as const;

const PREVIEW_BUCKETS = (() => {
  let cumulativeDamage = 0;
  return TEAM_DPS.map((teamDps, index) => {
    const damage = Math.round(teamDps * 2);
    cumulativeDamage += damage;
    return {
      start: index * 2,
      end: index * 2 + 2,
      teamDps,
      damage,
      hits: String(teamDps === 0 ? 0 : 8 + (index % 7)),
      cumulativeDamage,
      roles: CHARACTER_DPS.map((values, roleIndex) => ({
        characterId: roleIndex + 1,
        dps: values[index] ?? 0,
      })),
    };
  });
})();

export const TIMELINE_UI_PREVIEW: TimelinePreviewSnapshot = {
  duration: 48,
  bucketSeconds: 2,
  effectiveBucketSeconds: 2,
  totalDamage: 5_688_311,
  peakDps: 176_000,
  timeStopIntervals: [
    { start: 15.5, end: 18.5 },
    { start: 34, end: 36 },
  ],
  markers: [
    { offset: 2, labelKey: "First Half", kind: "half" },
    { offset: 27, labelKey: "Second Half", kind: "half" },
    { offset: 46, labelKey: "Clear", kind: "clear" },
  ],
  characters: [
    { id: 1, name: "真红", color: "#ef4444", totalDamage: 3_132_330 },
    { id: 2, name: "哈索尔", color: "#f59e0b", totalDamage: 1_150_840 },
    { id: 3, name: "娜娜莉", color: "#8b5cf6", totalDamage: 821_141 },
  ],
  buckets: PREVIEW_BUCKETS,
};
