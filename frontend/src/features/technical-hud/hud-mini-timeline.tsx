import { useEffect, useMemo, useRef, useState, type PointerEvent } from "react";

import { t, tf } from "@/lib/i18n";
import type { HudTimelineSnapshot } from "@/lib/tauri/technical-contract";
import { cn } from "@/lib/utils";

import { formatHudDuration, formatHudNumber } from "./technical-view-model";
import {
  buildMiniTimelineDrawModel,
  canvasBackingStoreSize,
  MINI_TIMELINE_HEIGHT,
  timelineBucketIndexAtX,
} from "./mini-timeline";

interface HudMiniTimelineProps {
  timeline: HudTimelineSnapshot;
  interactive: boolean;
}

interface CanvasSize {
  width: number;
  height: number;
  devicePixelRatio: number;
}

const EMPTY_SIZE: CanvasSize = {
  width: 0,
  height: MINI_TIMELINE_HEIGHT,
  devicePixelRatio: 1,
};

export function HudMiniTimeline({
  timeline,
  interactive,
}: HudMiniTimelineProps) {
  const containerRef = useRef<HTMLElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState<CanvasSize>(EMPTY_SIZE);
  const [hoveredBucketIndex, setHoveredBucketIndex] = useState<number | null>(
    null,
  );
  const drawModel = useMemo(
    () =>
      buildMiniTimelineDrawModel(timeline, size.width, MINI_TIMELINE_HEIGHT),
    [size.width, timeline],
  );

  useEffect(() => {
    const container = containerRef.current;
    if (container === null) {
      return;
    }

    const measure = () => {
      const next: CanvasSize = {
        width: container.getBoundingClientRect().width,
        height: MINI_TIMELINE_HEIGHT,
        devicePixelRatio: Math.max(window.devicePixelRatio, 1),
      };
      setSize((current) =>
        current.width === next.width &&
        current.height === next.height &&
        current.devicePixelRatio === next.devicePixelRatio
          ? current
          : next,
      );
    };
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    window.addEventListener("resize", measure);
    measure();

    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas === null || size.width <= 0 || size.height <= 0) {
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      const context = canvas.getContext("2d", { alpha: true });
      if (context === null) {
        return;
      }

      const backingStore = canvasBackingStoreSize(
        size.width,
        size.height,
        size.devicePixelRatio,
      );
      if (
        canvas.width !== backingStore.pixelWidth ||
        canvas.height !== backingStore.pixelHeight
      ) {
        canvas.width = backingStore.pixelWidth;
        canvas.height = backingStore.pixelHeight;
      }
      context.setTransform(
        backingStore.devicePixelRatio,
        0,
        0,
        backingStore.devicePixelRatio,
        0,
        0,
      );
      context.clearRect(0, 0, size.width, size.height);
      drawTimeline(context, canvas, drawModel);
    });

    return () => window.cancelAnimationFrame(frame);
  }, [drawModel, size]);

  const hoveredPoint =
    hoveredBucketIndex === null
      ? null
      : (drawModel.points[hoveredBucketIndex] ?? null);
  const hoveredBucket =
    hoveredBucketIndex === null
      ? null
      : (timeline.buckets[hoveredBucketIndex] ?? null);
  const tooltipLeft =
    hoveredPoint === null
      ? 0
      : Math.min(Math.max(hoveredPoint.x, 78), Math.max(78, size.width - 78));

  const updateHoveredBucket = (event: PointerEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    setHoveredBucketIndex(
      timelineBucketIndexAtX(drawModel, event.clientX - bounds.left),
    );
  };

  return (
    <section
      ref={containerRef}
      className="relative h-[42px] w-full"
      aria-label={t("Timeline")}
    >
      <canvas
        ref={canvasRef}
        className="pointer-events-none absolute inset-0 size-full"
        style={{ color: "var(--hud-role-1)" }}
        aria-hidden="true"
      />

      <div className="pointer-events-none absolute inset-x-1 top-0 flex items-center justify-between text-[9px] text-white/55">
        <span className="hud-text-halo">{t("DPS")}</span>
        <span className="hud-text-halo">
          {tf("Peak DPS: {0}", [formatHudNumber(timeline.peakDps)])}
        </span>
      </div>

      <p className="sr-only">
        {tf("Peak DPS: {0}", [formatHudNumber(timeline.peakDps)])}
      </p>

      <div
        className={cn(
          "absolute inset-0",
          interactive ? "pointer-events-auto" : "pointer-events-none",
        )}
        onPointerMove={interactive ? updateHoveredBucket : undefined}
        onPointerLeave={
          interactive ? () => setHoveredBucketIndex(null) : undefined
        }
        aria-hidden="true"
      />

      {interactive && hoveredPoint !== null && hoveredBucket !== null ? (
        <>
          <span
            className="pointer-events-none absolute top-3 bottom-1 w-px bg-cyan-100/65"
            style={{ left: hoveredPoint.x }}
            aria-hidden="true"
          />
          <div
            role="tooltip"
            className="pointer-events-none absolute bottom-1 z-10 -translate-x-1/2 rounded-sm bg-black/85 px-1.5 py-1 text-[9px] leading-3 text-white shadow-sm ring-1 ring-white/15"
            style={{ left: tooltipLeft }}
          >
            <p className="whitespace-nowrap text-white/70">
              {tf("Time: {0}–{1}", [
                formatHudDuration(hoveredBucket.startSeconds),
                formatHudDuration(hoveredBucket.endSeconds),
              ])}
            </p>
            <p className="whitespace-nowrap">
              {tf("DPS: {0}", [formatHudNumber(hoveredBucket.dps)])}
              {" · "}
              {tf("Damage: {0}", [formatHudNumber(hoveredBucket.damage)])}
            </p>
          </div>
        </>
      ) : null}
    </section>
  );
}

function drawTimeline(
  context: CanvasRenderingContext2D,
  canvas: HTMLCanvasElement,
  model: ReturnType<typeof buildMiniTimelineDrawModel>,
) {
  context.save();
  context.lineCap = "round";
  context.lineJoin = "round";

  context.beginPath();
  context.moveTo(0, model.baselineY);
  context.lineTo(model.width, model.baselineY);
  context.strokeStyle = "rgb(255 255 255 / 22%)";
  context.lineWidth = 1;
  context.stroke();

  if (model.points.length > 0) {
    strokeTimeline(context, model, "rgb(0 0 0 / 78%)", 3.5);
    const accent = getComputedStyle(canvas).color || "#67e8f9";
    strokeTimeline(context, model, accent, 1.5);
  }

  context.restore();
}

function strokeTimeline(
  context: CanvasRenderingContext2D,
  model: ReturnType<typeof buildMiniTimelineDrawModel>,
  color: string,
  width: number,
) {
  context.beginPath();
  for (const [index, point] of model.points.entries()) {
    if (index === 0) {
      context.moveTo(point.x, point.y);
    } else {
      context.lineTo(point.x, point.y);
    }
  }
  context.strokeStyle = color;
  context.lineWidth = width;
  context.stroke();

  if (model.points.length === 1) {
    const point = model.points[0];
    context.beginPath();
    context.arc(point.x, point.y, width, 0, Math.PI * 2);
    context.fillStyle = color;
    context.fill();
  }
}
