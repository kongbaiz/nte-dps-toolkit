import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
  type PointerEvent,
} from "react";

import { dismissLayerWhenClosed } from "@/components/ui/layer-behavior";
import {
  ContextMenu,
  ContextMenuItem,
  ContextMenuPopup,
  ContextMenuPortal,
  ContextMenuPositioner,
  ContextMenuTrigger,
} from "@/components/ui/menu";
import { t, tf } from "@/lib/i18n";
import { cn } from "@/lib/utils";

import {
  addTimelineMarker,
  buildTimelineChartModel,
  clampTimelineZoom,
  intersectTimelineRange,
  normalizeTimelineRange,
  removeNearestTimelineMarker,
  timelineBucketIndexAtX,
  timelineCanvasBackingSize,
  timelineTimeToX,
  timelineXToTime,
  type TimelineCurveMode,
  type TimelinePreviewSnapshot,
  type TimelineRange,
} from "./timeline-ui-model";

interface TimelineChartProps {
  mode: TimelineCurveMode;
  selectedCharacterId: number | null;
  snapshot: TimelinePreviewSnapshot;
}

interface ChartSize {
  width: number;
  height: number;
  devicePixelRatio: number;
}

interface TimelineContextMenu {
  time: number;
}

const EMPTY_SIZE: ChartSize = { width: 0, height: 0, devicePixelRatio: 1 };

export function TimelineChart({
  mode,
  selectedCharacterId,
  snapshot,
}: TimelineChartProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState(EMPTY_SIZE);
  const [hoveredBucketIndex, setHoveredBucketIndex] = useState<number | null>(
    null,
  );
  const [dragAnchor, setDragAnchor] = useState<number | null>(null);
  const [dragSelection, setDragSelection] = useState<TimelineRange | null>(
    null,
  );
  const [zoom, setZoom] = useState<TimelineRange | null>(null);
  const [userMarkers, setUserMarkers] = useState<number[]>([]);
  const [contextMenu, setContextMenu] = useState<TimelineContextMenu | null>(
    null,
  );
  const effectiveZoom = useMemo(
    () => clampTimelineZoom(zoom, snapshot.duration),
    [snapshot.duration, zoom],
  );
  const model = useMemo(
    () =>
      buildTimelineChartModel(
        snapshot,
        size.width,
        size.height,
        mode,
        selectedCharacterId,
        effectiveZoom,
      ),
    [
      effectiveZoom,
      mode,
      selectedCharacterId,
      size.height,
      size.width,
      snapshot,
    ],
  );

  const selection = dragSelection
    ? normalizeTimelineRange(
        dragSelection[0],
        dragSelection[1],
        snapshot.duration,
      )
    : null;
  const zoomSelection = clampTimelineZoom(selection, snapshot.duration);
  const visibleSelection = selection
    ? intersectTimelineRange(selection, model.viewRange)
    : null;

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const update = () => {
      const bounds = container.getBoundingClientRect();
      setSize({
        width: Math.max(1, Math.round(bounds.width)),
        height: Math.max(1, Math.round(bounds.height)),
        devicePixelRatio: window.devicePixelRatio || 1,
      });
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context || size.width <= 0 || size.height <= 0) return;
    const frame = window.requestAnimationFrame(() => {
      const backing = timelineCanvasBackingSize(
        size.width,
        size.height,
        size.devicePixelRatio,
      );
      canvas.width = backing.width;
      canvas.height = backing.height;
      canvas.style.width = `${size.width}px`;
      canvas.style.height = `${size.height}px`;
      context.setTransform(
        size.devicePixelRatio,
        0,
        0,
        size.devicePixelRatio,
        0,
        0,
      );
      drawChart(context, snapshot, model, hoveredBucketIndex, userMarkers);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [hoveredBucketIndex, model, size, snapshot, userMarkers]);

  useEffect(() => {
    if (contextMenu === null) return;
    const closeContextMenuOnWindowBlur = () => setContextMenu(null);
    window.addEventListener("blur", closeContextMenuOnWindowBlur);
    return () => {
      window.removeEventListener("blur", closeContextMenuOnWindowBlur);
    };
  }, [contextMenu]);

  const hoveredBucket =
    hoveredBucketIndex === null
      ? null
      : (snapshot.buckets[hoveredBucketIndex] ?? null);
  const hoveredTime = hoveredBucket
    ? (hoveredBucket.start + hoveredBucket.end) / 2
    : 0;
  const hoveredX = hoveredBucket ? timelineTimeToX(model, hoveredTime) : 0;
  const hoveredDps = hoveredBucket
    ? mode === "team"
      ? hoveredBucket.teamDps
      : hoveredBucket.roles.reduce((peak, role) => Math.max(peak, role.dps), 0)
    : 0;
  const hoveredTimeStop = hoveredBucket
    ? (snapshot.timeStopIntervals.find(
        (interval) =>
          hoveredTime >= interval.start && hoveredTime <= interval.end,
      ) ?? null)
    : null;

  const updateHover = (event: PointerEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    const x = event.clientX - bounds.left;
    const y = event.clientY - bounds.top;
    setHoveredBucketIndex(
      y < model.plot.top || y > model.plot.top + model.plot.height
        ? null
        : timelineBucketIndexAtX(model, snapshot.buckets, x),
    );
    if (dragAnchor !== null) {
      setDragSelection([dragAnchor, timelineXToTime(model, x)]);
    }
  };

  const beginSelection = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    const x = event.clientX - bounds.left;
    const y = event.clientY - bounds.top;
    if (
      x < model.plot.left ||
      x > model.plot.left + model.plot.width ||
      y < model.plot.top ||
      y > model.plot.top + model.plot.height
    ) {
      return;
    }
    event.currentTarget.setPointerCapture(event.pointerId);
    setContextMenu(null);
    setDragAnchor(timelineXToTime(model, x));
    setDragSelection(null);
  };

  const finishSelection = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || dragAnchor === null) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    setDragSelection([
      dragAnchor,
      timelineXToTime(model, event.clientX - bounds.left),
    ]);
    setDragAnchor(null);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const openContextMenu = (event: MouseEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    const x = event.clientX - bounds.left;
    const y = event.clientY - bounds.top;
    if (
      x < model.plot.left ||
      x > model.plot.left + model.plot.width ||
      y < model.plot.top ||
      y > model.plot.top + model.plot.height
    ) {
      setContextMenu(null);
      return;
    }
    setContextMenu({
      time: timelineXToTime(model, x),
    });
  };

  return (
    <ContextMenu
      open={contextMenu !== null}
      onOpenChange={(open) =>
        dismissLayerWhenClosed(open, () => setContextMenu(null))
      }
    >
      <ContextMenuTrigger
        render={
          <div
            ref={containerRef}
            className="timeline-chart-shell relative min-h-64 w-full touch-none flex-[1_1_26rem] overflow-hidden rounded-xl"
            onContextMenu={openContextMenu}
            onPointerDown={beginSelection}
            onPointerMove={updateHover}
            onPointerUp={finishSelection}
            onPointerCancel={() => setDragAnchor(null)}
            onPointerLeave={() => setHoveredBucketIndex(null)}
          >
            <canvas
              ref={canvasRef}
              className="timeline-chart-canvas pointer-events-none absolute inset-0 size-full text-primary"
              aria-hidden="true"
            />
            <div className="timeline-chart-glass pointer-events-none absolute inset-x-3 top-2 z-[2] flex items-center justify-between gap-4 rounded-lg border px-2.5 py-1.5 text-xs">
              <span className="font-mono font-semibold text-primary">
                {t(mode === "team" ? "Peak DPS" : "Peak Character DPS")}:{" "}
                {number(model.maxDps)}
              </span>
              <span className="font-mono text-muted-foreground">
                {t("Total Damage")}: {number(snapshot.totalDamage)}
              </span>
            </div>
            <div className="pointer-events-none absolute inset-x-0 top-[4.25rem] z-[1] h-12 bg-gradient-to-b from-card/55 to-transparent" />
            {snapshot.markers
              .filter(
                (marker) =>
                  marker.offset >= model.viewRange[0] &&
                  marker.offset <= model.viewRange[1],
              )
              .map((marker) => (
                <span
                  key={`${marker.kind}-${marker.offset}`}
                  className={cn(
                    "timeline-marker-label pointer-events-none absolute top-11 z-[3] rounded-full border px-1.5 py-0.5 font-mono text-[9px] tracking-wide",
                    marker.offset <= snapshot.bucketSeconds * 0.5
                      ? "translate-x-0"
                      : marker.offset >=
                          model.viewRange[1] - snapshot.bucketSeconds * 0.5
                        ? "-translate-x-full"
                        : "-translate-x-1/2",
                    marker.kind === "clear"
                      ? "border-emerald-500/35 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300"
                      : "border-border bg-background text-muted-foreground",
                  )}
                  style={{
                    left: timelineTimeToX(model, marker.offset),
                  }}
                >
                  {t(marker.labelKey)}
                </span>
              ))}
            {snapshot.timeStopIntervals.map((interval, index) => {
              const visible = intersectTimelineRange(
                [interval.start, interval.end],
                model.viewRange,
              );
              if (visible === null) return null;
              const left = timelineTimeToX(model, visible[0]);
              const width = timelineTimeToX(model, visible[1]) - left;
              if (width < 36) return null;
              return (
                <span
                  key={`${interval.start}-${interval.end}-${index}`}
                  className="pointer-events-none absolute z-[3] -translate-x-1/2 rounded-full border border-amber-500/25 bg-amber-500/10 px-1.5 py-0.5 font-mono text-[9px] text-amber-700 dark:text-amber-300"
                  style={{
                    left: left + width / 2,
                    top: model.plot.top + 7,
                    maxWidth: Math.max(28, width - 8),
                  }}
                >
                  {(interval.end - interval.start).toFixed(1)}s
                </span>
              );
            })}
            {userMarkers.map((marker, index) => {
              if (marker < model.viewRange[0] || marker > model.viewRange[1]) {
                return null;
              }
              const left = timelineTimeToX(model, marker);
              return (
                <div
                  key={`${marker}-${index}`}
                  className="pointer-events-none absolute z-[3] border-l border-primary/70"
                  style={{
                    left,
                    top: model.plot.top,
                    height: model.plot.height,
                  }}
                >
                  <span className="absolute top-1 left-1 max-w-36 whitespace-nowrap rounded-full border border-primary/30 bg-background/90 px-1.5 py-0.5 font-mono text-[9px] text-primary shadow-sm">
                    {tf("Marker {} · {}s", [
                      String(index + 1),
                      formatSeconds(marker),
                    ])}
                  </span>
                </div>
              );
            })}
            {visibleSelection && (
              <div
                className="pointer-events-none absolute z-[2] border-x border-primary/75 bg-primary/12"
                style={{
                  left: timelineTimeToX(model, visibleSelection[0]),
                  top: model.plot.top,
                  width:
                    timelineTimeToX(model, visibleSelection[1]) -
                    timelineTimeToX(model, visibleSelection[0]),
                  height: model.plot.height,
                }}
              >
                <span className="absolute top-2 left-1/2 -translate-x-1/2 whitespace-nowrap rounded-full bg-background/90 px-2 py-0.5 text-[9px] text-primary shadow-sm">
                  {tf("Selection · {}s - {}s", [
                    formatSeconds(selection?.[0] ?? 0),
                    formatSeconds(selection?.[1] ?? 0),
                  ])}
                </span>
              </div>
            )}
            {hoveredBucket && (
              <>
                <div
                  className="timeline-hover-lens pointer-events-none absolute bottom-7 z-[1] w-16 -translate-x-1/2"
                  style={{ left: hoveredX, top: model.plot.top }}
                />
                <div
                  className="pointer-events-none absolute bottom-1.5 z-[4] -translate-x-1/2 rounded-full border bg-background px-2 py-0.5 font-mono text-[10px] shadow-sm"
                  style={{ left: hoveredX }}
                >
                  {hoveredTime.toFixed(1)}s
                </div>
              </>
            )}
            {hoveredBucket && (
              <div
                className={cn(
                  "timeline-tooltip pointer-events-none absolute top-20 z-10 min-w-48 rounded-lg border bg-popover p-3 text-xs text-popover-foreground shadow-lg",
                  hoveredX > size.width - 224 ? "right-3" : "left-3",
                )}
                style={
                  hoveredX > size.width - 224
                    ? undefined
                    : { left: hoveredX + 12 }
                }
              >
                <p className="mb-1 font-mono font-semibold">
                  {hoveredTimeStop
                    ? t("Time-stop Interval")
                    : `${formatSeconds(hoveredBucket.start)}s – ${formatSeconds(hoveredBucket.end)}s`}
                </p>
                <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-muted-foreground">
                  {hoveredTimeStop && (
                    <>
                      <dt>{t("Range")}</dt>
                      <dd className="text-right font-mono text-foreground">
                        {formatSeconds(hoveredTimeStop.start)}s –{" "}
                        {formatSeconds(hoveredTimeStop.end)}s
                      </dd>
                      <dt>{t("Duration")}</dt>
                      <dd className="text-right font-mono text-foreground">
                        {formatSeconds(
                          hoveredTimeStop.end - hoveredTimeStop.start,
                        )}
                        s
                      </dd>
                      <dt>{t("Current Bucket")}</dt>
                      <dd className="text-right font-mono text-foreground">
                        {formatSeconds(hoveredBucket.start)}s –{" "}
                        {formatSeconds(hoveredBucket.end)}s
                      </dd>
                    </>
                  )}
                  <dt>{t(mode === "team" ? "DPS" : "Top Character DPS")}</dt>
                  <dd className="text-right font-mono text-foreground">
                    {number(hoveredDps)}
                  </dd>
                  <dt>{t("Damage")}</dt>
                  <dd className="text-right font-mono text-foreground">
                    {number(hoveredBucket.damage)}
                  </dd>
                  <dt>{t("Hits")}</dt>
                  <dd className="text-right font-mono text-foreground">
                    {hoveredBucket.hits}
                  </dd>
                  <dt>{t("Cumulative")}</dt>
                  <dd className="text-right font-mono text-foreground">
                    {number(hoveredBucket.cumulativeDamage)}
                  </dd>
                </dl>
                <div className="mt-2.5 space-y-1.5 border-t pt-2">
                  {hoveredBucket.roles.map((role) => {
                    const character = snapshot.characters.find(
                      (candidate) => candidate.id === role.characterId,
                    );
                    const ratio =
                      hoveredDps > 0 ? (role.dps / hoveredDps) * 100 : 0;
                    return (
                      <div
                        key={role.characterId}
                        className="grid grid-cols-[4rem_1fr_3.5rem] items-center gap-2"
                      >
                        <span className="truncate">
                          {character?.name ?? "—"}
                        </span>
                        <span className="h-1 overflow-hidden rounded-full bg-muted">
                          <span
                            className="block h-full rounded-full"
                            style={{
                              width: `${ratio}%`,
                              backgroundColor: character?.color,
                            }}
                          />
                        </span>
                        <span className="text-right font-mono text-muted-foreground">
                          {number(role.dps)}
                        </span>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}
            <p className="sr-only">
              {t("Timeline")}. {t("Peak DPS")}: {number(model.maxDps)}.{" "}
              {t("Total Damage")}: {number(snapshot.totalDamage)}.
            </p>
          </div>
        }
      />
      {contextMenu !== null && (
        <ContextMenuPortal>
          <ContextMenuPositioner>
            <ContextMenuPopup
              aria-label={t("Timeline")}
              className="flex w-48 flex-col gap-0.5 p-1.5"
            >
              <TimelineMenuButton
                label="Add marker here"
                onClick={() => {
                  setUserMarkers((current) =>
                    addTimelineMarker(current, contextMenu.time),
                  );
                }}
              />
              <TimelineMenuButton
                disabled={zoomSelection === null}
                label="Zoom to selection"
                onClick={() => {
                  setZoom(zoomSelection);
                  setDragSelection(null);
                }}
              />
              <TimelineMenuButton
                disabled={effectiveZoom === null}
                label="Reset zoom"
                onClick={() => setZoom(null)}
              />
              <TimelineMenuButton
                disabled={userMarkers.length === 0}
                label="Remove nearest marker"
                onClick={() => {
                  setUserMarkers((current) =>
                    removeNearestTimelineMarker(current, contextMenu.time),
                  );
                }}
              />
            </ContextMenuPopup>
          </ContextMenuPositioner>
        </ContextMenuPortal>
      )}
    </ContextMenu>
  );
}

function drawChart(
  context: CanvasRenderingContext2D,
  snapshot: TimelinePreviewSnapshot,
  model: ReturnType<typeof buildTimelineChartModel>,
  hoveredBucketIndex: number | null,
  userMarkers: readonly number[],
) {
  const { plot } = model;
  context.clearRect(0, 0, context.canvas.width, context.canvas.height);
  const styles = getComputedStyle(context.canvas);
  const border = styles.getPropertyValue("--border").trim() || "#d4d4d8";
  const muted =
    styles.getPropertyValue("--muted-foreground").trim() || "#71717a";
  const foreground =
    styles.getPropertyValue("--foreground").trim() || "#18181b";
  const warning = "#f59e0b";

  context.save();
  context.lineWidth = 1;
  context.strokeStyle = color(border, 0.75);
  context.fillStyle = muted;
  context.font = "10px ui-monospace, monospace";
  context.textAlign = "right";
  context.textBaseline = "middle";
  for (let step = 0; step <= 4; step += 1) {
    const y = plot.top + (plot.height * step) / 4;
    context.beginPath();
    context.moveTo(plot.left, y);
    context.lineTo(plot.left + plot.width, y);
    context.stroke();
    context.fillText(number(model.maxDps * (1 - step / 4)), plot.left - 8, y);
  }
  context.textAlign = "center";
  context.textBaseline = "top";
  const visibleDuration = model.viewRange[1] - model.viewRange[0];
  for (let step = 0; step <= 4; step += 1) {
    const x = plot.left + (plot.width * step) / 4;
    context.beginPath();
    context.moveTo(x, plot.top);
    context.lineTo(x, plot.top + plot.height);
    context.stroke();
    context.fillText(
      `${Math.round(model.viewRange[0] + (visibleDuration * step) / 4)}s`,
      x,
      plot.top + plot.height + 8,
    );
  }

  for (const interval of snapshot.timeStopIntervals) {
    const visible = intersectTimelineRange(
      [interval.start, interval.end],
      model.viewRange,
    );
    if (visible === null) continue;
    const left = timelineTimeToX(model, visible[0]);
    const right = timelineTimeToX(model, visible[1]);
    const band = context.createLinearGradient(left, plot.top, right, plot.top);
    band.addColorStop(0, color(warning, 0.05));
    band.addColorStop(0.5, color(warning, 0.18));
    band.addColorStop(1, color(warning, 0.05));
    context.fillStyle = band;
    context.fillRect(left, plot.top, right - left, plot.height);
    context.save();
    context.beginPath();
    context.rect(left, plot.top, right - left, plot.height);
    context.clip();
    context.strokeStyle = color(warning, 0.12);
    context.lineWidth = 1;
    for (let x = left - plot.height; x < right + plot.height; x += 12) {
      context.beginPath();
      context.moveTo(x, plot.top + plot.height);
      context.lineTo(x + plot.height, plot.top);
      context.stroke();
    }
    context.restore();
  }

  for (const marker of snapshot.markers) {
    if (
      marker.offset < model.viewRange[0] ||
      marker.offset > model.viewRange[1]
    ) {
      continue;
    }
    const x = timelineTimeToX(model, marker.offset);
    const markerColor =
      marker.kind === "clear"
        ? "#22c55e"
        : marker.kind === "exit"
          ? "#ef4444"
          : styles.getPropertyValue("--primary").trim() || "#18181b";
    context.strokeStyle = color(markerColor, 0.85);
    context.setLineDash([4, 4]);
    context.beginPath();
    context.moveTo(x, plot.top);
    context.lineTo(x, plot.top + plot.height);
    context.stroke();
    context.setLineDash([]);
  }

  context.strokeStyle = color(
    styles.getPropertyValue("--primary").trim() || foreground,
    0.72,
  );
  for (const marker of userMarkers) {
    if (marker < model.viewRange[0] || marker > model.viewRange[1]) continue;
    const x = timelineTimeToX(model, marker);
    context.beginPath();
    context.moveTo(x, plot.top);
    context.lineTo(x, plot.top + plot.height);
    context.stroke();
  }

  for (const line of model.lines) {
    if (line.points.length === 0) continue;
    const lineColor = resolveCanvasColor(line.color, styles, foreground);
    if (line.fill) {
      const area = context.createLinearGradient(
        0,
        plot.top,
        0,
        plot.top + plot.height,
      );
      area.addColorStop(0, color(lineColor, line.muted ? 0.015 : 0.1));
      area.addColorStop(1, color(lineColor, 0));
      context.fillStyle = area;
      context.globalAlpha = line.muted ? 0.12 : 0.5;
      context.beginPath();
      context.moveTo(line.points[0].x, plot.top + plot.height);
      line.points.forEach((point) => context.lineTo(point.x, point.y));
      context.lineTo(
        line.points[line.points.length - 1].x,
        plot.top + plot.height,
      );
      context.closePath();
      context.fill();
    }

    context.strokeStyle = lineColor;
    context.globalAlpha = line.muted ? 0.2 : 0.95;
    context.lineWidth = line.muted ? 1.25 : line.width;
    context.lineJoin = "round";
    context.lineCap = "round";
    context.beginPath();
    line.points.forEach((point, index) => {
      if (index === 0) context.moveTo(point.x, point.y);
      else context.lineTo(point.x, point.y);
    });
    context.shadowColor = lineColor;
    context.shadowBlur = line.muted || line.kind === "cumulative" ? 0 : 7;
    context.stroke();
    context.shadowBlur = 0;
    context.globalAlpha = line.muted ? 0.2 : 1;
    context.lineWidth = line.muted ? 1 : 1.25;
    context.stroke();
  }
  context.globalAlpha = 1;

  if (hoveredBucketIndex !== null) {
    const point = model.lines[0]?.points.find(
      (candidate) => candidate.bucketIndex === hoveredBucketIndex,
    );
    if (point) {
      context.strokeStyle = color(foreground, 0.55);
      context.lineWidth = 1;
      context.beginPath();
      context.moveTo(point.x, plot.top);
      context.lineTo(point.x, plot.top + plot.height);
      context.stroke();
      for (const line of model.lines) {
        if (line.kind !== "dps") continue;
        const indexedPoint = line.points.find(
          (candidate) => candidate.bucketIndex === hoveredBucketIndex,
        );
        if (!indexedPoint) continue;
        const pointColor = resolveCanvasColor(line.color, styles, foreground);
        context.fillStyle = pointColor;
        context.shadowColor = pointColor;
        context.shadowBlur = 8;
        context.beginPath();
        context.arc(indexedPoint.x, indexedPoint.y, 3.25, 0, Math.PI * 2);
        context.fill();
      }
      context.shadowBlur = 0;
    }
  }
  context.restore();
}

function color(value: string, alpha: number): string {
  if (value.startsWith("#") && value.length === 7) {
    const red = Number.parseInt(value.slice(1, 3), 16);
    const green = Number.parseInt(value.slice(3, 5), 16);
    const blue = Number.parseInt(value.slice(5, 7), 16);
    return `rgb(${red} ${green} ${blue} / ${alpha})`;
  }
  if (value.startsWith("oklch(") && !value.includes("/")) {
    return `${value.slice(0, -1)} / ${alpha})`;
  }
  return value;
}

function resolveCanvasColor(
  value: string,
  styles: CSSStyleDeclaration,
  fallback: string,
): string {
  const variable = /^var\((--[^)]+)\)$/.exec(value)?.[1];
  return variable
    ? styles.getPropertyValue(variable).trim() || fallback
    : value;
}

function TimelineMenuButton({
  label,
  disabled = false,
  onClick,
}: {
  label: string;
  disabled?: boolean;
  onClick(): void;
}) {
  return (
    <ContextMenuItem
      className="disabled:cursor-default disabled:hover:bg-transparent"
      disabled={disabled}
      onClick={onClick}
    >
      {t(label)}
    </ContextMenuItem>
  );
}

function formatSeconds(value: number): string {
  return Math.abs(value) >= 10 ? value.toFixed(0) : value.toFixed(1);
}

function number(value: number): string {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(
    value,
  );
}
