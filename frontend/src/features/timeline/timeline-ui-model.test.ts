import { describe, expect, it } from "vitest";

import { TIMELINE_UI_PREVIEW } from "./timeline-ui-preview";
import {
  addTimelineMarker,
  buildTimelineChartModel,
  clampTimelineZoom,
  normalizeTimelineRange,
  removeNearestTimelineMarker,
  timelineBucketIndexAtX,
  timelineCanvasBackingSize,
  timelineTimeToX,
  timelineXToTime,
} from "./timeline-ui-model";

describe("Timeline UI model", () => {
  it("keeps the plot inside narrow and wide containers", () => {
    for (const width of [320, 820, 1440]) {
      const model = buildTimelineChartModel(
        TIMELINE_UI_PREVIEW,
        width,
        360,
        "characters",
        null,
      );
      expect(model.plot.left).toBeGreaterThan(0);
      expect(model.plot.top).toBeGreaterThanOrEqual(72);
      expect(model.plot.left + model.plot.width).toBeLessThan(width);
      expect(model.lines).toHaveLength(TIMELINE_UI_PREVIEW.characters.length);
      expect(model.maxDps).toBeGreaterThanOrEqual(TIMELINE_UI_PREVIEW.peakDps);
    }
  });

  it("retains the team cumulative-damage curve", () => {
    const model = buildTimelineChartModel(
      TIMELINE_UI_PREVIEW,
      800,
      360,
      "team",
      null,
    );
    expect(model.lines.map((line) => line.kind)).toEqual(["dps", "cumulative"]);
  });

  it("maps pointer positions to bounded buckets", () => {
    const model = buildTimelineChartModel(
      TIMELINE_UI_PREVIEW,
      800,
      360,
      "team",
      null,
    );
    expect(
      timelineBucketIndexAtX(
        model,
        TIMELINE_UI_PREVIEW.buckets,
        model.plot.left,
      ),
    ).toBe(0);
    expect(
      timelineBucketIndexAtX(
        model,
        TIMELINE_UI_PREVIEW.buckets,
        model.plot.left + model.plot.width,
      ),
    ).toBe(TIMELINE_UI_PREVIEW.buckets.length - 1);
    expect(
      timelineBucketIndexAtX(
        model,
        TIMELINE_UI_PREVIEW.buckets,
        model.plot.left - 1,
      ),
    ).toBeNull();
  });

  it("round-trips pointer time and clamps zoom like the egui timeline", () => {
    const model = buildTimelineChartModel(
      TIMELINE_UI_PREVIEW,
      800,
      360,
      "team",
      null,
      [10, 30],
    );
    const x = timelineTimeToX(model, 20);
    expect(timelineXToTime(model, x)).toBeCloseTo(20);
    expect(normalizeTimelineRange(8, 2, 10)).toEqual([2, 8]);
    expect(normalizeTimelineRange(-4, 14, 10)).toEqual([0, 10]);
    expect(normalizeTimelineRange(3, 3, 10)).toBeNull();
    expect(clampTimelineZoom([0, 10], 10)).toBeNull();
    expect(clampTimelineZoom([12, 8], 10)).toEqual([8, 10]);
  });

  it("sorts user markers and removes the nearest marker", () => {
    let markers = addTimelineMarker([], 8);
    markers = addTimelineMarker(markers, 2);
    markers = addTimelineMarker(markers, 5);
    expect(markers).toEqual([2, 5, 8]);
    expect(removeNearestTimelineMarker(markers, 6)).toEqual([2, 8]);
  });

  it("uses a DPR-aware canvas backing store", () => {
    expect(timelineCanvasBackingSize(640, 320, 1.5)).toEqual({
      width: 960,
      height: 480,
    });
  });
});
