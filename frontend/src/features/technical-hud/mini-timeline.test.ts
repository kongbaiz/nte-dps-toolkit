import { describe, expect, it } from "vitest";

import type { HudTimelineSnapshot } from "@/lib/tauri/technical-contract";

import {
  buildMiniTimelineDrawModel,
  canvasBackingStoreSize,
  timelineBucketIndexAtX,
} from "./mini-timeline";

const timeline: HudTimelineSnapshot = {
  bucketSeconds: 1,
  durationSeconds: 3,
  peakDps: 300,
  buckets: [
    {
      startSeconds: 0,
      endSeconds: 1,
      damage: 100,
      dps: 100,
      hits: "1",
    },
    {
      startSeconds: 1,
      endSeconds: 2,
      damage: 300,
      dps: 300,
      hits: "3",
    },
    {
      startSeconds: 2,
      endSeconds: 3,
      damage: 150,
      dps: 150,
      hits: "2",
    },
  ],
};

describe("mini timeline draw model", () => {
  it("maps bucket time and peak DPS into bounded CSS-pixel coordinates", () => {
    const model = buildMiniTimelineDrawModel(timeline, 300, 42);

    expect(model.baselineY).toBe(37);
    expect(model.points.map((point) => point.x)).toEqual([100, 200, 300]);
    expect(model.points[1].y).toBe(13);
    expect(model.points.every((point) => point.y <= model.baselineY)).toBe(
      true,
    );
  });

  it("maps pointer positions to bucket intervals and clamps chart edges", () => {
    const model = buildMiniTimelineDrawModel(timeline, 300, 42);

    expect(timelineBucketIndexAtX(model, -10)).toBe(0);
    expect(timelineBucketIndexAtX(model, 150)).toBe(1);
    expect(timelineBucketIndexAtX(model, 400)).toBe(2);
    expect(timelineBucketIndexAtX(model, Number.NaN)).toBeNull();
  });

  it("keeps invalid viewport dimensions out of the canvas model", () => {
    const model = buildMiniTimelineDrawModel(
      { ...timeline, buckets: [] },
      Number.NaN,
      -1,
    );

    expect(model.width).toBe(0);
    expect(model.height).toBe(0);
    expect(model.points).toEqual([]);
    expect(timelineBucketIndexAtX(model, 0)).toBeNull();
  });

  it("scales the backing store for fractional Windows DPI", () => {
    expect(canvasBackingStoreSize(300, 42, 1.25)).toEqual({
      pixelWidth: 375,
      pixelHeight: 53,
      devicePixelRatio: 1.25,
    });
    expect(canvasBackingStoreSize(300, 42, 1.5)).toEqual({
      pixelWidth: 450,
      pixelHeight: 63,
      devicePixelRatio: 1.5,
    });
    expect(canvasBackingStoreSize(Number.NaN, -1, Number.NaN)).toEqual({
      pixelWidth: 1,
      pixelHeight: 1,
      devicePixelRatio: 1,
    });
  });
});
