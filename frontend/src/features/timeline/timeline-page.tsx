import {
  Activity,
  Clock3,
  MousePointer2,
  RefreshCw,
  TimerReset,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useState } from "react";

import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { t } from "@/lib/i18n";
import type {
  TimelineCurveMode,
  TimelineScope,
} from "@/lib/tauri/timeline-contract";
import { cn } from "@/lib/utils";

import { TimelineChart } from "./timeline-chart";
import { useTimeline } from "./use-timeline";

const SCOPES: Array<{ id: TimelineScope; labelKey: string }> = [
  { id: "all", labelKey: "Whole Battle" },
  { id: "upper", labelKey: "First Half" },
  { id: "lower", labelKey: "Second Half" },
];

export function TimelinePage() {
  const {
    state,
    scope,
    setScope,
    pending,
    mutationError,
    retry,
    setPreferences,
  } = useTimeline();
  const snapshot = state.status === "ready" ? state.snapshot : null;
  const mode = snapshot?.viewMode ?? "team";
  const [draftBucketSeconds, setDraftBucketSeconds] = useState(1);
  const [selectedCharacterId, setSelectedCharacterId] = useState<number | null>(
    null,
  );

  useEffect(() => {
    if (snapshot) setDraftBucketSeconds(snapshot.bucketSeconds);
  }, [snapshot]);

  useEffect(() => {
    if (
      selectedCharacterId !== null &&
      !snapshot?.characters.some(
        (character) => character.id === selectedCharacterId,
      )
    ) {
      setSelectedCharacterId(null);
    }
  }, [selectedCharacterId, snapshot]);

  const commitBucket = () => {
    if (
      snapshot &&
      Math.abs(draftBucketSeconds - snapshot.bucketSeconds) > 0.0001
    ) {
      void setPreferences(draftBucketSeconds, mode);
    }
  };

  const changeMode = (nextMode: TimelineCurveMode) => {
    if (!snapshot || nextMode === mode) return;
    setSelectedCharacterId(null);
    void setPreferences(snapshot.bucketSeconds, nextMode);
  };

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b bg-background px-3 py-3 min-[640px]:px-5">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-base font-semibold">{t("Timeline")}</h1>
            <Badge variant="outline" className="gap-1.5">
              <span className="size-1.5 rounded-full bg-emerald-500" />
              {t("Live")}
            </Badge>
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t("Timeline follows the current capture")}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-1 rounded-lg bg-muted/55 p-1">
          {SCOPES.map((item) => (
            <Button
              key={item.id}
              size="sm"
              aria-pressed={scope === item.id}
              variant={scope === item.id ? "default" : "ghost"}
              className={scope === item.id ? "hover:bg-primary" : undefined}
              disabled={pending}
              onClick={() => setScope(item.id)}
            >
              {t(item.labelKey)}
            </Button>
          ))}
        </div>
      </header>

      <div className="timeline-scrollport min-h-0 flex-1 overflow-y-auto p-3 min-[900px]:p-4">
        <div className="timeline-content flex min-h-full w-full flex-col gap-3.5">
          {mutationError && state.status !== "error" && (
            <Alert variant="destructive">
              <AlertTitle>{t("Timeline update failed")}</AlertTitle>
              <AlertDescription>{t(mutationError.messageKey)}</AlertDescription>
            </Alert>
          )}

          <div className="flex flex-wrap items-center gap-x-5 gap-y-3 border-y border-border/70 bg-background/25 px-2 py-2.5">
            <label className="flex min-w-[16rem] flex-1 items-center gap-3 text-sm">
              <span className="whitespace-nowrap text-muted-foreground">
                {t("Bucket Interval")}
              </span>
              <input
                aria-label={t("Bucket Interval")}
                className="h-1.5 min-w-24 flex-1 cursor-pointer accent-primary disabled:cursor-not-allowed"
                type="range"
                min={snapshot?.bucketSecondsMin ?? 0.2}
                max={snapshot?.bucketSecondsMax ?? 10}
                step={snapshot?.bucketSecondsStep ?? 0.1}
                value={draftBucketSeconds}
                disabled={!snapshot || pending}
                onChange={(event) =>
                  setDraftBucketSeconds(Number(event.currentTarget.value))
                }
                onPointerUp={commitBucket}
                onKeyUp={commitBucket}
                onBlur={commitBucket}
              />
              <span className="w-12 text-right font-mono text-xs">
                {draftBucketSeconds.toFixed(1)}s
              </span>
            </label>
            <div className="h-5 w-px bg-border max-[700px]:hidden" />
            <div className="flex items-center gap-2 text-sm">
              <span className="text-muted-foreground">{t("Curve")}</span>
              <div className="flex rounded-md bg-muted/55 p-0.5">
                {(["team", "characters"] as const).map((item) => (
                  <Button
                    key={item}
                    size="sm"
                    aria-pressed={mode === item}
                    variant={mode === item ? "default" : "ghost"}
                    className={mode === item ? "hover:bg-primary" : undefined}
                    disabled={!snapshot || pending}
                    onClick={() => changeMode(item)}
                  >
                    {t(item === "team" ? "Team" : "Characters")}
                  </Button>
                ))}
              </div>
            </div>
          </div>

          {state.status === "loading" && <TimelineLoading />}
          {state.status === "error" && (
            <Alert variant="destructive" className="timeline-panel">
              <Activity className="size-4" aria-hidden="true" />
              <AlertTitle>{t("Timeline data could not be loaded")}</AlertTitle>
              <AlertDescription>{t(state.error.messageKey)}</AlertDescription>
              <AlertAction>
                <Button size="sm" variant="outline" onClick={retry}>
                  <RefreshCw className="size-3.5" aria-hidden="true" />
                  {t("Retry")}
                </Button>
              </AlertAction>
            </Alert>
          )}
          {snapshot && !snapshot.hasData && <TimelineEmpty />}
          {snapshot?.hasData && (
            <>
              <div className="timeline-metrics grid grid-cols-2 border-y border-border/70 bg-background/25 min-[780px]:grid-cols-4">
                <Metric
                  icon={MousePointer2}
                  label="Total Damage"
                  value={number(snapshot.totalDamage)}
                  prominent
                />
                <Metric
                  icon={Clock3}
                  label="Peak DPS"
                  value={number(snapshot.peakDps)}
                  prominent
                />
                <Metric
                  icon={TimerReset}
                  label="Combat Time"
                  value={`${snapshot.duration.toFixed(1)}s`}
                />
                <Metric
                  icon={TimerReset}
                  label="Time-stop Intervals"
                  value={snapshot.timeStopIntervals.length.toString()}
                />
              </div>

              {snapshot.segments.length > 1 && (
                <div className="flex flex-wrap items-center gap-2 px-1 py-1 text-xs">
                  <span className="text-muted-foreground">
                    {t("Combat segments")}
                  </span>
                  {snapshot.segments.map((segment, index) => (
                    <div
                      key={`${segment.start}-${segment.end}`}
                      className="timeline-segment group flex items-center gap-2 rounded-full bg-muted/55 py-1 pr-3 pl-1.5"
                    >
                      <span className="flex size-5 items-center justify-center rounded-full bg-primary/12 font-mono font-semibold text-primary">
                        {index + 1}
                      </span>
                      <span>
                        {formatSeconds(segment.start)}s–
                        {formatSeconds(segment.end)}s
                      </span>
                      <span className="font-mono font-semibold">
                        {number(segment.dps)} DPS
                      </span>
                    </div>
                  ))}
                </div>
              )}

              {mode === "characters" && (
                <div className="flex flex-wrap items-center gap-1.5">
                  {snapshot.characters.map((character) => {
                    const selected = selectedCharacterId === character.id;
                    return (
                      <button
                        key={character.id}
                        type="button"
                        aria-pressed={selected}
                        onClick={() =>
                          setSelectedCharacterId(selected ? null : character.id)
                        }
                        className={cn(
                          "timeline-legend flex h-8 items-center gap-2 rounded-md px-2.5 text-xs transition-colors hover:bg-muted/70",
                          selected &&
                            "bg-primary text-primary-foreground hover:bg-primary hover:text-primary-foreground",
                          selectedCharacterId !== null &&
                            !selected &&
                            "opacity-45",
                        )}
                      >
                        <span className="relative flex size-3 items-center justify-center">
                          <span
                            className="absolute size-3 rounded-full opacity-30 blur-[3px]"
                            style={{ backgroundColor: character.color }}
                          />
                          <span
                            className="relative size-2 rounded-full"
                            style={{ backgroundColor: character.color }}
                          />
                        </span>
                        <span>{character.name}</span>
                        <span
                          className={cn(
                            "font-mono text-muted-foreground",
                            selected && "text-primary-foreground/75",
                          )}
                        >
                          {number(character.totalDamage)}
                        </span>
                      </button>
                    );
                  })}
                </div>
              )}

              <p className="text-xs text-muted-foreground">
                {t(
                  "Click a legend to highlight; drag across the chart to select a range; right-click for markers and zoom",
                )}
              </p>
              <TimelineChart
                mode={mode}
                selectedCharacterId={selectedCharacterId}
                snapshot={snapshot}
              />
              <footer className="flex shrink-0 flex-wrap items-center justify-between gap-2 px-1 pt-0.5 pb-2 text-xs text-muted-foreground">
                <span>
                  {t("Retained window")} · {snapshot.duration.toFixed(1)}s ·{" "}
                  {snapshot.bucketSeconds.toFixed(1)}s{" "}
                  {t("Bucket Interval").toLowerCase()}
                </span>
                <span>
                  {snapshot.buckets.length} {t("samples")} ·{" "}
                  {snapshot.markers.length} {t("event markers")}
                </span>
              </footer>
            </>
          )}
        </div>
      </div>
    </section>
  );
}

function TimelineLoading() {
  return (
    <div
      className="flex flex-1 flex-col gap-3"
      aria-label={t("Loading Timeline")}
    >
      <div className="grid grid-cols-2 gap-2.5 min-[780px]:grid-cols-4">
        {Array.from({ length: 4 }, (_, index) => (
          <Skeleton key={index} className="h-20 rounded-xl" />
        ))}
      </div>
      <Skeleton className="min-h-64 flex-1 rounded-2xl" />
    </div>
  );
}

function TimelineEmpty() {
  return (
    <Empty className="timeline-panel min-h-72 flex-1 border">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Activity aria-hidden="true" />
        </EmptyMedia>
        <EmptyTitle>{t("No timeline data yet")}</EmptyTitle>
        <EmptyDescription>
          {t("Start capture or import a replay to populate this chart.")}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}

function Metric({
  icon: Icon,
  label,
  value,
  prominent = false,
}: {
  icon: LucideIcon;
  label: string;
  value: string;
  prominent?: boolean;
}) {
  return (
    <div className="timeline-metric flex min-w-0 items-center gap-3 px-3 py-3">
      <div className="timeline-metric-icon flex size-9 shrink-0 items-center justify-center rounded-lg text-muted-foreground">
        <Icon className="size-4" aria-hidden="true" />
      </div>
      <div className="min-w-0">
        <p className="truncate text-xs text-muted-foreground">{t(label)}</p>
        <p
          className={cn(
            "truncate font-mono font-semibold",
            prominent && "text-primary",
          )}
        >
          {value}
        </p>
      </div>
    </div>
  );
}

function number(value: number): string {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(
    value,
  );
}

function formatSeconds(value: number): string {
  return value >= 10 ? value.toFixed(0) : value.toFixed(1);
}
