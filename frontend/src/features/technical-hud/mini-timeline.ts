import type { HudTimelineSnapshot } from "@/lib/tauri/technical-contract";

export const MINI_TIMELINE_HEIGHT = 42;
const MINI_TIMELINE_TOP_INSET = 13;
const MINI_TIMELINE_BOTTOM_INSET = 5;

export interface MiniTimelinePoint {
  bucketIndex: number;
  startX: number;
  endX: number;
  x: number;
  y: number;
}

export interface MiniTimelineDrawModel {
  width: number;
  height: number;
  baselineY: number;
  peakDps: number;
  durationSeconds: number;
  points: MiniTimelinePoint[];
}

export interface CanvasBackingStoreSize {
  pixelWidth: number;
  pixelHeight: number;
  devicePixelRatio: number;
}

export function buildMiniTimelineDrawModel(
  timeline: HudTimelineSnapshot,
  width: number,
  height: number,
): MiniTimelineDrawModel {
  const safeWidth = finiteDimension(width);
  const safeHeight = finiteDimension(height);
  const baselineY = Math.max(
    0,
    safeHeight - Math.min(MINI_TIMELINE_BOTTOM_INSET, safeHeight),
  );
  const graphTop = Math.min(MINI_TIMELINE_TOP_INSET, baselineY);
  const graphHeight = Math.max(0, baselineY - graphTop);
  const durationSeconds = Math.max(
    timeline.durationSeconds,
    timeline.buckets.at(-1)?.endSeconds ?? 0,
    Number.EPSILON,
  );
  const peakDps = Math.max(
    timeline.peakDps,
    ...timeline.buckets.map((bucket) => bucket.dps),
    Number.EPSILON,
  );
  const points = timeline.buckets.map((bucket, bucketIndex) => {
    const startRatio = clamp(bucket.startSeconds / durationSeconds, 0, 1);
    const endRatio = clamp(bucket.endSeconds / durationSeconds, startRatio, 1);
    const dpsRatio = clamp(bucket.dps / peakDps, 0, 1);
    const startX = safeWidth * startRatio;
    const endX = safeWidth * endRatio;

    return {
      bucketIndex,
      startX,
      endX,
      x: endX,
      y: baselineY - graphHeight * dpsRatio,
    };
  });

  return {
    width: safeWidth,
    height: safeHeight,
    baselineY,
    peakDps,
    durationSeconds,
    points,
  };
}

export function timelineBucketIndexAtX(
  model: MiniTimelineDrawModel,
  pointerX: number,
): number | null {
  if (model.points.length === 0 || !Number.isFinite(pointerX)) {
    return null;
  }

  const x = clamp(pointerX, 0, model.width);
  return (
    model.points.find((point) => x >= point.startX && x <= point.endX)
      ?.bucketIndex ??
    model.points.reduce((nearest, point) => {
      const nearestDistance = Math.abs(model.points[nearest].x - x);
      return Math.abs(point.x - x) < nearestDistance
        ? point.bucketIndex
        : nearest;
    }, 0)
  );
}

export function canvasBackingStoreSize(
  width: number,
  height: number,
  devicePixelRatio: number,
): CanvasBackingStoreSize {
  const safeRatio =
    Number.isFinite(devicePixelRatio) && devicePixelRatio > 0
      ? devicePixelRatio
      : 1;
  return {
    pixelWidth: Math.max(1, Math.round(finiteDimension(width) * safeRatio)),
    pixelHeight: Math.max(1, Math.round(finiteDimension(height) * safeRatio)),
    devicePixelRatio: safeRatio,
  };
}

function finiteDimension(value: number): number {
  return Number.isFinite(value) ? Math.max(0, value) : 0;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), maximum);
}
