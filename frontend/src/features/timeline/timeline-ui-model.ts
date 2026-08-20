import type {
  TimelineCurveMode,
  TimelineScope,
} from "@/lib/tauri/timeline-contract";

export type { TimelineCurveMode, TimelineScope };

export interface TimelinePreviewBucket {
  start: number;
  end: number;
  teamDps: number;
  damage: number;
  hits: string;
  cumulativeDamage: number;
  roles: Array<{ characterId: number; dps: number }>;
}

export interface TimelinePreviewCharacter {
  id: number;
  name: string;
  color: string;
  totalDamage: number;
}

export interface TimelinePreviewMarker {
  offset: number;
  labelKey: string;
  kind: "half" | "clear" | "exit";
}

export interface TimelinePreviewSnapshot {
  duration: number;
  bucketSeconds: number;
  effectiveBucketSeconds: number;
  totalDamage: number;
  peakDps: number;
  timeStopIntervals: Array<{ start: number; end: number }>;
  markers: TimelinePreviewMarker[];
  characters: TimelinePreviewCharacter[];
  buckets: TimelinePreviewBucket[];
}

export interface TimelineChartPoint {
  x: number;
  y: number;
  bucketIndex: number;
}

export interface TimelineChartLine {
  id: string;
  kind: "dps" | "cumulative";
  color: string;
  muted: boolean;
  fill: boolean;
  width: number;
  points: TimelineChartPoint[];
}

export type TimelineRange = readonly [start: number, end: number];

export interface TimelineChartModel {
  plot: { left: number; top: number; width: number; height: number };
  maxDps: number;
  viewRange: TimelineRange;
  lines: TimelineChartLine[];
}

const CHART_LEFT = 54;
const CHART_RIGHT = 16;
// Reserve separate HTML lanes for the summary strip and event labels. Canvas
// curves start below both, so a peak at maxDps is never hidden by an overlay.
const CHART_TOP = 72;
const CHART_BOTTOM = 30;

export function buildTimelineChartModel(
  snapshot: TimelinePreviewSnapshot,
  width: number,
  height: number,
  mode: TimelineCurveMode,
  selectedCharacterId: number | null,
  requestedViewRange?: TimelineRange | null,
): TimelineChartModel {
  const plot = {
    left: CHART_LEFT,
    top: CHART_TOP,
    width: Math.max(1, width - CHART_LEFT - CHART_RIGHT),
    height: Math.max(1, height - CHART_TOP - CHART_BOTTOM),
  };
  const duration = Math.max(snapshot.duration, 0.001);
  const viewRange =
    clampTimelineZoom(requestedViewRange ?? null, duration) ??
    ([0, duration] as const);
  const rolePeak = snapshot.buckets.reduce(
    (peak, bucket) => Math.max(peak, ...bucket.roles.map((role) => role.dps)),
    0,
  );
  const maxDps = Math.max(
    1,
    mode === "team" ? snapshot.peakDps : Math.max(rolePeak, snapshot.peakDps),
  );
  const point = (
    value: number,
    scale: number,
    bucketIndex: number,
  ): TimelineChartPoint => {
    const bucket = snapshot.buckets[bucketIndex];
    const time = bucket ? (bucket.start + bucket.end) / 2 : 0;
    return {
      x: timelineTimeToX({ plot, viewRange }, time),
      y: plot.top + plot.height - (value / Math.max(scale, 1)) * plot.height,
      bucketIndex,
    };
  };
  const visibleBuckets = snapshot.buckets
    .map((bucket, index) => ({
      bucket,
      index,
      time: (bucket.start + bucket.end) / 2,
    }))
    .filter(({ time }) => time >= viewRange[0] && time <= viewRange[1]);

  const lines: TimelineChartLine[] =
    mode === "team"
      ? [
          {
            id: "team",
            kind: "dps",
            color: "var(--primary)",
            muted: false,
            fill: true,
            width: 2.25,
            points: visibleBuckets.map(({ bucket, index }) =>
              point(bucket.teamDps, maxDps, index),
            ),
          },
          {
            id: "cumulative",
            kind: "cumulative",
            color: "var(--muted-foreground)",
            muted: false,
            fill: false,
            width: 1.5,
            points: visibleBuckets.map(({ bucket, index }) =>
              point(bucket.cumulativeDamage, snapshot.totalDamage, index),
            ),
          },
        ]
      : snapshot.characters.map((character) => ({
          id: String(character.id),
          kind: "dps" as const,
          color: character.color,
          muted:
            selectedCharacterId !== null &&
            selectedCharacterId !== character.id,
          fill: true,
          width: selectedCharacterId === character.id ? 3.4 : 1.5,
          points: visibleBuckets.map(({ bucket, index }) => {
            const role = bucket.roles.find(
              (candidate) => candidate.characterId === character.id,
            );
            return point(role?.dps ?? 0, maxDps, index);
          }),
        }));

  return { plot, maxDps, viewRange, lines };
}

export function timelineBucketIndexAtX(
  model: TimelineChartModel,
  buckets: readonly TimelinePreviewBucket[],
  x: number,
): number | null {
  if (buckets.length === 0 || x < model.plot.left) return null;
  if (x > model.plot.left + model.plot.width) return null;
  const time = timelineXToTime(model, x);
  const containing = buckets.findIndex(
    (bucket) => time >= bucket.start && time <= bucket.end,
  );
  if (containing >= 0) return containing;
  return buckets.reduce(
    (nearest, bucket, index) => {
      const distance = Math.abs((bucket.start + bucket.end) / 2 - time);
      return distance < nearest.distance ? { index, distance } : nearest;
    },
    { index: 0, distance: Number.POSITIVE_INFINITY },
  ).index;
}

export function timelineXToTime(
  model: Pick<TimelineChartModel, "plot" | "viewRange">,
  x: number,
): number {
  const progress = Math.min(
    1,
    Math.max(0, (x - model.plot.left) / model.plot.width),
  );
  return (
    model.viewRange[0] + (model.viewRange[1] - model.viewRange[0]) * progress
  );
}

export function timelineTimeToX(
  model: Pick<TimelineChartModel, "plot" | "viewRange">,
  time: number,
): number {
  const duration = model.viewRange[1] - model.viewRange[0];
  const progress = Math.min(
    1,
    Math.max(0, (time - model.viewRange[0]) / Math.max(duration, 0.001)),
  );
  return model.plot.left + model.plot.width * progress;
}

export function normalizeTimelineRange(
  start: number,
  end: number,
  duration: number,
): TimelineRange | null {
  if (
    !Number.isFinite(start) ||
    !Number.isFinite(end) ||
    !Number.isFinite(duration) ||
    duration <= 0
  ) {
    return null;
  }
  const lower = Math.min(start, end);
  const upper = Math.max(start, end);
  const range = [
    Math.min(duration, Math.max(0, lower)),
    Math.min(duration, Math.max(0, upper)),
  ] as const;
  return range[1] - range[0] > Number.EPSILON ? range : null;
}

export function clampTimelineZoom(
  range: TimelineRange | null,
  duration: number,
): TimelineRange | null {
  if (range === null) return null;
  const normalized = normalizeTimelineRange(range[0], range[1], duration);
  if (normalized === null) return null;
  return normalized[1] - normalized[0] < duration - Number.EPSILON
    ? normalized
    : null;
}

export function intersectTimelineRange(
  range: TimelineRange,
  viewRange: TimelineRange,
): TimelineRange | null {
  const lower = Math.max(range[0], viewRange[0]);
  const upper = Math.min(range[1], viewRange[1]);
  return upper > lower ? [lower, upper] : null;
}

export function addTimelineMarker(
  markers: readonly number[],
  time: number,
): number[] {
  return [...markers, time].sort((left, right) => left - right);
}

export function removeNearestTimelineMarker(
  markers: readonly number[],
  time: number,
): number[] {
  if (markers.length === 0) return [];
  let nearest = 0;
  for (let index = 1; index < markers.length; index += 1) {
    if (Math.abs(markers[index] - time) < Math.abs(markers[nearest] - time)) {
      nearest = index;
    }
  }
  return markers.filter((_, index) => index !== nearest);
}

export function timelineCanvasBackingSize(
  width: number,
  height: number,
  devicePixelRatio: number,
) {
  const ratio = Math.max(1, devicePixelRatio);
  return {
    width: Math.max(1, Math.round(width * ratio)),
    height: Math.max(1, Math.round(height * ratio)),
  };
}
