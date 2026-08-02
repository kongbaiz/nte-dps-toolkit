import {
  Activity,
  ChevronRight,
  Radio,
  RefreshCw,
  Search,
  TriangleAlert,
  X,
} from "lucide-react";
import { useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";

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
import { Switch } from "@/components/ui/switch";
import { t, tf } from "@/lib/i18n";
import type {
  PacketSnapshot,
  PacketsCapturePhase,
  PacketsCommandError,
  PacketsSnapshot,
} from "@/lib/tauri/packets-contract";
import { cn } from "@/lib/utils";

import {
  formatPacketTimestamp,
  normalizePacketSearch,
  packetMatches,
  packetsContentKind,
} from "./packets-model";
import { usePackets } from "./use-packets";

const PHASE_LABELS: Record<PacketsCapturePhase, string> = {
  idle: "Capture idle",
  starting: "Capture starting",
  running: "Live",
  stopping: "Capture stopping",
  stopped: "Capture stopped",
  failed: "Capture failed",
};

export function PacketsPage() {
  const { state, streamError, clearStreamError, retry } = usePackets();
  const [query, setQuery] = useState("");
  const [hitsOnly, setHitsOnly] = useState(false);
  const deferredQuery = useDeferredValue(query);
  const viewportRef = useRef<HTMLDivElement>(null);
  const followLatestRef = useRef(true);
  const snapshot = state.status === "ready" ? state.snapshot : null;
  const packetGeneration = snapshot?.packetGeneration;
  const filteredPackets = useMemo(() => {
    if (!snapshot) return [];
    const normalizedQuery = normalizePacketSearch(deferredQuery);
    return snapshot.packets.filter((packet) =>
      packetMatches(packet, normalizedQuery, hitsOnly),
    );
  }, [deferredQuery, hitsOnly, snapshot]);
  const contentKind = packetsContentKind(
    state.status,
    snapshot?.packets.length ?? 0,
    filteredPackets.length,
  );

  useEffect(() => {
    if (packetGeneration === undefined || !followLatestRef.current) return;
    const frame = requestAnimationFrame(() => {
      const viewport = viewportRef.current;
      if (viewport) viewport.scrollTop = viewport.scrollHeight;
    });
    return () => cancelAnimationFrame(frame);
  }, [packetGeneration]);

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background">
      {streamError ? (
        <PacketsStreamError error={streamError} onDismiss={clearStreamError} />
      ) : null}
      <header className="flex flex-wrap items-center justify-between gap-3 border-b px-3 py-3 min-[640px]:px-5">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-base font-semibold">{t("Packets")}</h1>
            {snapshot ? <CaptureBadge phase={snapshot.capturePhase} /> : null}
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t("Inspect decoded packets from the current capture in real time")}
          </p>
        </div>
        {snapshot ? (
          <span className="text-xs text-muted-foreground tabular-nums">
            {tf("Showing the latest {0} of {1} retained packets", [
              snapshot.packets.length.toLocaleString(),
              snapshot.retainedPacketCount.toLocaleString(),
            ])}
          </span>
        ) : null}
      </header>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b bg-muted/15 px-3 py-2.5 min-[640px]:px-5">
        <label className="relative min-w-[220px] flex-1 max-w-xl">
          <Search
            className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
          <span className="sr-only">{t("Search")}</span>
          <input
            className="h-9 w-full rounded-lg border bg-background pr-3 pl-9 text-sm outline-none transition-shadow placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/25"
            placeholder={t("IP / ID / protocol name")}
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
          />
        </label>
        <label className="flex h-9 cursor-pointer items-center gap-2 text-sm whitespace-nowrap">
          <Switch
            size="sm"
            checked={hitsOnly}
            aria-label={t("Hit packets only")}
            onCheckedChange={setHitsOnly}
          />
          {t("Hit packets only")}
        </label>
        {snapshot ? <PacketCounters snapshot={snapshot} /> : null}
      </div>

      <div
        ref={viewportRef}
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain"
        onScroll={(event) => {
          const viewport = event.currentTarget;
          followLatestRef.current =
            viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <
            72;
        }}
      >
        {contentKind === "loading" ? <PacketsLoading /> : null}
        {contentKind === "error" && state.status === "error" ? (
          <PacketsLoadError error={state.error} onRetry={retry} />
        ) : null}
        {contentKind === "empty" ? <PacketsEmpty /> : null}
        {contentKind === "filtered-empty" ? <PacketsFilteredEmpty /> : null}
        {contentKind === "list" ? (
          <div className="mx-auto w-full max-w-[1800px] divide-y">
            {filteredPackets.map((packet) => (
              <PacketRow key={packet.sequence} packet={packet} />
            ))}
          </div>
        ) : null}
      </div>
    </section>
  );
}

function CaptureBadge({ phase }: { phase: PacketsCapturePhase }) {
  const active = phase === "running";
  return (
    <Badge variant="outline" className="gap-1.5">
      <span
        className={cn(
          "size-1.5 rounded-full",
          active ? "bg-emerald-500" : "bg-muted-foreground/55",
          phase === "failed" && "bg-destructive",
        )}
      />
      {t(PHASE_LABELS[phase])}
    </Badge>
  );
}

function PacketCounters({ snapshot }: { snapshot: PacketsSnapshot }) {
  return (
    <dl className="ml-auto flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[11px] text-muted-foreground tabular-nums">
      <div className="flex items-center gap-1">
        <dt>{t("Events")}</dt>
        <dd className="text-foreground">
          {snapshot.eventCount.toLocaleString()}
        </dd>
      </div>
      <div className="flex items-center gap-1">
        <dt>{t("Packets")}</dt>
        <dd className="text-foreground">{snapshot.observedPacketCount}</dd>
      </div>
      <div className="flex items-center gap-1">
        <dt>{t("Queued")}</dt>
        <dd className="text-foreground">
          {snapshot.queuedEventCount.toLocaleString()}
        </dd>
      </div>
    </dl>
  );
}

function PacketRow({ packet }: { packet: PacketSnapshot }) {
  const ids = packet.declaredIds.join(", ");
  return (
    <details
      className="group px-3 py-1.5 open:bg-muted/20 min-[640px]:px-5"
      style={{ contentVisibility: "auto", containIntrinsicSize: "64px" }}
    >
      <summary className="flex min-h-11 cursor-pointer list-none items-center gap-2 rounded-md px-2 py-1.5 text-sm outline-none hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring/45 [&::-webkit-details-marker]:hidden">
        <ChevronRight
          className="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-90"
          aria-hidden="true"
        />
        <time className="w-[92px] shrink-0 font-mono text-xs text-muted-foreground tabular-nums">
          {formatPacketTimestamp(packet.timestamp)}
        </time>
        <DirectionBadge direction={packet.direction} />
        <span
          className="min-w-[180px] flex-1 truncate font-mono text-xs"
          title={`${packet.source} -> ${packet.destination}`}
        >
          {packet.source} <span className="text-muted-foreground">→</span>{" "}
          {packet.destination}
        </span>
        <span className="hidden shrink-0 font-mono text-[11px] text-muted-foreground tabular-nums min-[820px]:inline">
          {packet.payloadLen.toLocaleString()} B
        </span>
        <span
          className="hidden max-w-[220px] shrink-0 truncate font-mono text-[11px] text-muted-foreground min-[1040px]:inline"
          title={ids}
        >
          ids=[{ids}]
        </span>
        <Badge
          variant={packet.parsedHits > 0 ? "default" : "secondary"}
          className="shrink-0 font-mono tabular-nums"
        >
          {tf("{0} hits", [packet.parsedHits.toLocaleString()])}
        </Badge>
      </summary>
      <div className="mr-2 mb-2 ml-8 border-l pl-4">
        {packet.note ? (
          <p className="mb-2 text-xs text-amber-700 dark:text-amber-300">
            {packet.note}
          </p>
        ) : null}
        <div className="mb-1.5 text-xs font-medium text-primary">
          {t("Auto Parse")}
        </div>
        <pre className="max-h-80 overflow-auto rounded-lg border bg-muted/35 p-3 font-mono text-xs leading-5 break-words whitespace-pre-wrap select-text">
          {packet.decodedText || t("No readable decoded text")}
        </pre>
      </div>
    </details>
  );
}

function DirectionBadge({ direction }: { direction: string }) {
  const normalized = direction.toLocaleLowerCase();
  const label =
    normalized === "outgoing"
      ? t("Outgoing")
      : normalized === "incoming"
        ? t("Incoming")
        : direction;
  return (
    <Badge variant="outline" className="w-[58px] shrink-0 justify-center">
      {label}
    </Badge>
  );
}

function PacketsLoading() {
  return (
    <div className="mx-auto flex w-full max-w-[1800px] flex-col gap-1 px-3 py-4 min-[640px]:px-5">
      {Array.from({ length: 10 }, (_, index) => (
        <Skeleton key={index} className="h-12 w-full rounded-md" />
      ))}
    </div>
  );
}

function PacketsLoadError({
  error,
  onRetry,
}: {
  error: PacketsCommandError;
  onRetry: () => void;
}) {
  return (
    <div className="mx-auto w-full max-w-3xl p-5">
      <Alert variant="destructive">
        <TriangleAlert className="size-4" aria-hidden="true" />
        <AlertTitle>{t("Packet data could not be loaded")}</AlertTitle>
        <AlertDescription>{t(error.messageKey)}</AlertDescription>
        <AlertAction>
          <Button size="sm" variant="outline" onClick={onRetry}>
            <RefreshCw className="size-3.5" aria-hidden="true" />
            {t("Retry")}
          </Button>
        </AlertAction>
      </Alert>
    </div>
  );
}

function PacketsEmpty() {
  return (
    <Empty className="min-h-full border-0">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Radio aria-hidden="true" />
        </EmptyMedia>
        <EmptyTitle>{t("No decoded packets yet")}</EmptyTitle>
        <EmptyDescription>
          {t("Start capture to inspect the latest decoded packet details.")}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}

function PacketsFilteredEmpty() {
  return (
    <Empty className="min-h-full border-0">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Search aria-hidden="true" />
        </EmptyMedia>
        <EmptyTitle>{t("No packets match the current filters")}</EmptyTitle>
        <EmptyDescription>
          {t("Clear the search or show packets without hits.")}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}

function PacketsStreamError({
  error,
  onDismiss,
}: {
  error: PacketsCommandError;
  onDismiss: () => void;
}) {
  return createPortal(
    <div className="pointer-events-none fixed inset-x-0 top-14 z-[90] flex justify-center px-4">
      <Alert
        variant="destructive"
        className="pointer-events-auto w-full max-w-xl shadow-xl"
      >
        <Activity className="size-4" aria-hidden="true" />
        <AlertTitle>{t("Packet stream was interrupted")}</AlertTitle>
        <AlertDescription>{t(error.messageKey)}</AlertDescription>
        <AlertAction>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("Dismiss")}
            onClick={onDismiss}
          >
            <X aria-hidden="true" />
          </Button>
        </AlertAction>
      </Alert>
    </div>,
    document.body,
  );
}
