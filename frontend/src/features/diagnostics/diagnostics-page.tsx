import {
  Activity,
  Check,
  CircleCheck,
  Clipboard,
  Download,
  FileJson,
  FileSearch,
  Gauge,
  Network,
  RefreshCw,
  Save,
  ShieldAlert,
  TriangleAlert,
  Upload,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
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
import { t, tf } from "@/lib/i18n";
import type {
  DiagnosticsCheckSnapshot,
  DiagnosticsCommandError,
  DiagnosticsQualitySnapshot,
  DiagnosticsSnapshot,
} from "@/lib/tauri/diagnostics-contract";
import { cn } from "@/lib/utils";

import {
  buildRedactedDiagnosticsReport,
  diagnosticsContentKind,
  formatByteCount,
  formatDecimalString,
} from "./diagnostics-model";
import { useDiagnostics, type DiagnosticsAction } from "./use-diagnostics";

const CAPTURE_PHASE_LABELS = {
  idle: "Capture idle",
  starting: "Capture starting",
  running: "Live",
  stopping: "Capture stopping",
  stopped: "Capture stopped",
  failed: "Capture failed",
} as const;

const STATUS_LABELS = {
  passed: "Passed",
  warning: "Warning",
  failed: "Failed",
} as const;

const SOURCE_LABELS = {
  live: "Live capture",
  pcapng_replay: "PCAPNG replay",
  json_replay: "JSON replay",
  unknown: "No capture data",
} as const;

export function DiagnosticsPage() {
  const {
    state,
    streamError,
    actionError,
    pendingAction,
    perform,
    clearNotice,
    retry,
  } = useDiagnostics();
  const [copyState, setCopyState] = useState<"idle" | "copied" | "error">(
    "idle",
  );
  const snapshot = state.status === "ready" ? state.snapshot : null;
  const contentKind = diagnosticsContentKind(state.status);
  const notice = actionError ?? streamError;
  const reportText = useMemo(
    () =>
      snapshot
        ? buildRedactedDiagnosticsReport(
            snapshot,
            translateDiagnosticToken,
            (key, arguments_) => tf(key, arguments_),
          )
        : "",
    [snapshot],
  );

  useEffect(() => {
    if (copyState === "idle") return;
    const timeout = window.setTimeout(() => setCopyState("idle"), 1_600);
    return () => window.clearTimeout(timeout);
  }, [copyState]);

  async function copyReport() {
    try {
      await navigator.clipboard.writeText(reportText);
      setCopyState("copied");
    } catch {
      setCopyState("error");
    }
  }

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background">
      {notice ? (
        <DiagnosticsNotice error={notice} onDismiss={clearNotice} />
      ) : null}
      <header className="flex flex-wrap items-center justify-between gap-3 border-b px-3 py-3 min-[640px]:px-5">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-base font-semibold">{t("Diagnostics")}</h1>
            {snapshot ? <CaptureBadge snapshot={snapshot} /> : null}
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t(
              "Inspect capture environment, runtime checks and parse quality without exposing private payloads",
            )}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={!snapshot?.report || copyState === "copied"}
            onClick={() => void copyReport()}
          >
            {copyState === "copied" ? (
              <Check aria-hidden="true" />
            ) : (
              <Clipboard aria-hidden="true" />
            )}
            {t(
              copyState === "copied"
                ? "Copied"
                : copyState === "error"
                  ? "Copy failed"
                  : "Copy Redacted Report",
            )}
          </Button>
          <Button
            size="sm"
            disabled={pendingAction !== null}
            onClick={() => void perform("run")}
          >
            <RefreshCw
              className={cn(pendingAction === "run" && "animate-spin")}
              aria-hidden="true"
            />
            {t(pendingAction === "run" ? "Detecting" : "Run Diagnostics")}
          </Button>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
        {contentKind === "loading" ? <DiagnosticsLoading /> : null}
        {contentKind === "error" && state.status === "error" ? (
          <DiagnosticsLoadError error={state.error} onRetry={retry} />
        ) : null}
        {contentKind === "ready" && snapshot ? (
          <div className="mx-auto w-full max-w-[1800px]">
            <EnvironmentSection snapshot={snapshot} />
            <HistoryArchiveWarning
              droppedCount={snapshot.capture.droppedHistoryArchives}
            />
            <FileActions
              snapshot={snapshot}
              pendingAction={pendingAction}
              perform={perform}
            />
            <ReportSection snapshot={snapshot} />
            <QualitySection quality={snapshot.quality} />
          </div>
        ) : null}
      </div>
    </section>
  );
}

function HistoryArchiveWarning({ droppedCount }: { droppedCount: string }) {
  if (droppedCount === "0") return null;
  return (
    <div className="border-b px-4 py-3">
      <Alert variant="destructive">
        <TriangleAlert aria-hidden="true" />
        <AlertTitle>{t("Automatic history archives were dropped")}</AlertTitle>
        <AlertDescription>
          {tf(
            "The retry queue dropped {0} archive(s); review diagnostics before relying on the history list.",
            [formatDecimalString(droppedCount)],
          )}
        </AlertDescription>
      </Alert>
    </div>
  );
}

function CaptureBadge({ snapshot }: { snapshot: DiagnosticsSnapshot }) {
  const active =
    snapshot.capture.phase === "running" || snapshot.capture.replayRunning;
  return (
    <Badge variant="outline" className="gap-1.5">
      <span
        className={cn(
          "size-1.5 rounded-full",
          active ? "bg-emerald-500" : "bg-muted-foreground/60",
        )}
        aria-hidden="true"
      />
      {t(
        snapshot.capture.replayRunning
          ? "Replay running"
          : CAPTURE_PHASE_LABELS[snapshot.capture.phase],
      )}
    </Badge>
  );
}

function EnvironmentSection({ snapshot }: { snapshot: DiagnosticsSnapshot }) {
  const environment = snapshot.environment;
  const raw = snapshot.capture.rawCapture;
  const connection = environment?.gameConnection;
  const rows = [
    {
      label: "Capture device",
      value: environment?.deviceLabel ?? t("Not detected"),
      detail: environment?.manualDevice
        ? t("Manually selected")
        : t("Automatic"),
    },
    {
      label: "Local IP",
      value: environment?.localIp ?? t("Not detected"),
      detail: snapshot.adapterVersion,
    },
    {
      label: "Game connection",
      value: connection
        ? `${connection.remoteIp}:${connection.remotePort.toLocaleString()}`
        : t("Not detected"),
      detail: connection
        ? tf("PID {0} · local {1}", [
            connection.pid.toString(),
            connection.localIp,
          ])
        : t("Enter a game scene before running diagnostics"),
    },
    {
      label: "Active BPF",
      value: snapshot.capture.activeFilter ?? t("Not active"),
      detail: t("The effective filter reported by the capture controller"),
    },
    {
      label: "Raw capture",
      value: raw
        ? tf("{0} packets · {1}", [
            formatDecimalString(raw.packetCount),
            formatByteCount(raw.capturedBytes),
          ])
        : t("No retained raw capture"),
      detail: raw?.writeError
        ? t("The raw capture writer reported an error")
        : (raw?.fileName ??
          t("A completed live capture can be saved as PCAPNG")),
    },
  ];
  return (
    <section aria-labelledby="diagnostics-environment">
      <SectionHeading
        icon={Network}
        id="diagnostics-environment"
        title="Capture Environment"
        description="The active adapter, connection and capture pipeline"
      />
      <dl className="grid border-y bg-card/25 min-[760px]:grid-cols-2 min-[1240px]:grid-cols-5">
        {rows.map((row) => (
          <div
            className="min-w-0 border-b px-4 py-3 min-[760px]:border-r min-[1240px]:border-b-0"
            key={row.label}
          >
            <dt className="text-xs text-muted-foreground">{t(row.label)}</dt>
            <dd className="mt-1 truncate text-sm font-medium" title={row.value}>
              {row.value}
            </dd>
            <dd
              className="mt-0.5 truncate text-[11px] text-muted-foreground"
              title={row.detail}
            >
              {row.detail}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

function FileActions({
  snapshot,
  pendingAction,
  perform,
}: {
  snapshot: DiagnosticsSnapshot;
  pendingAction: DiagnosticsAction | null;
  perform: (action: DiagnosticsAction) => Promise<boolean>;
}) {
  const actions: Array<{
    action: DiagnosticsAction;
    label: string;
    icon: typeof Upload;
    enabled: boolean;
  }> = [
    {
      action: "import-pcapng",
      label: "Import PCAPNG",
      icon: Upload,
      enabled: snapshot.actions.canImport,
    },
    {
      action: "import-json",
      label: "Import Capture JSON",
      icon: FileJson,
      enabled: snapshot.actions.canImport,
    },
    {
      action: "export-json",
      label: "Export Parsed JSON",
      icon: Download,
      enabled: snapshot.actions.canExportParsed,
    },
    {
      action: "export-pcapng",
      label: "Save Full PCAPNG As",
      icon: Save,
      enabled: snapshot.actions.canExportRaw,
    },
  ];
  return (
    <section className="border-b px-4 py-3">
      <div className="flex flex-wrap items-center gap-2">
        {actions.map(({ action, label, icon: Icon, enabled }) => (
          <Button
            key={action}
            variant="outline"
            size="sm"
            disabled={!enabled || pendingAction !== null}
            onClick={() => void perform(action)}
          >
            <Icon
              className={cn(pendingAction === action && "animate-pulse")}
              aria-hidden="true"
            />
            {t(label)}
          </Button>
        ))}
        <p className="min-w-[260px] flex-1 text-xs text-muted-foreground min-[920px]:text-right">
          {t(
            "Importing a capture starts replay through the same reducer and clears current combat statistics",
          )}
        </p>
      </div>
    </section>
  );
}

function ReportSection({ snapshot }: { snapshot: DiagnosticsSnapshot }) {
  const report = snapshot.report;
  return (
    <section aria-labelledby="diagnostics-report">
      <SectionHeading
        icon={ShieldAlert}
        id="diagnostics-report"
        title="Runtime Diagnostics"
        description="Bounded checks for capture readiness and parser state"
      >
        {report ? (
          <div className="flex items-center gap-3 text-xs tabular-nums">
            <span className="text-destructive">
              {tf("{0} failed", [report.failedCount.toLocaleString()])}
            </span>
            <span className="text-amber-700 dark:text-amber-300">
              {tf("{0} warnings", [report.warningCount.toLocaleString()])}
            </span>
            <span className="text-muted-foreground">
              {tf("{0} checks", [report.checks.length.toLocaleString()])}
            </span>
          </div>
        ) : null}
      </SectionHeading>
      {report ? (
        <div className="divide-y border-y">
          {report.checks.map((check, index) => (
            <DiagnosticCheckRow
              check={check}
              key={`${check.titleKey}:${index}`}
            />
          ))}
        </div>
      ) : (
        <Empty className="min-h-52 border-y">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <FileSearch aria-hidden="true" />
            </EmptyMedia>
            <EmptyTitle>{t("Diagnostics have not been run yet")}</EmptyTitle>
            <EmptyDescription>
              {t(
                "Run diagnostics after entering a game scene to inspect the active capture environment.",
              )}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
    </section>
  );
}

function DiagnosticCheckRow({ check }: { check: DiagnosticsCheckSnapshot }) {
  const Icon = check.status === "passed" ? CircleCheck : TriangleAlert;
  return (
    <article className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 px-4 py-3 hover:bg-muted/20 min-[860px]:grid-cols-[auto_minmax(160px,0.55fr)_minmax(260px,1fr)_minmax(240px,0.8fr)] min-[860px]:items-start">
      <Icon
        className={cn(
          "mt-0.5 size-4",
          check.status === "passed"
            ? "text-emerald-600"
            : check.status === "warning"
              ? "text-amber-600"
              : "text-destructive",
        )}
        aria-hidden="true"
      />
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <h3 className="truncate text-sm font-medium">{t(check.titleKey)}</h3>
          <Badge variant="outline" className="shrink-0 text-[10px]">
            {t(STATUS_LABELS[check.status])}
          </Badge>
        </div>
      </div>
      <p className="col-start-2 text-xs text-muted-foreground min-[860px]:col-start-auto">
        {tf(check.detail.messageKey, check.detail.messageArguments)}
      </p>
      <p className="col-start-2 text-xs min-[860px]:col-start-auto">
        {tf(check.suggestion.messageKey, check.suggestion.messageArguments)}
      </p>
    </article>
  );
}

function QualitySection({ quality }: { quality: DiagnosticsQualitySnapshot }) {
  const rows = [
    [
      "Packets / packets with hits",
      quality.packetCount,
      quality.packetsWithHits,
    ],
    [
      "Hits / outgoing hits",
      quality.hitCount,
      formatDecimalString(quality.outgoingHits),
    ],
    [
      "Outgoing / incoming damage",
      quality.outgoingDamage,
      quality.incomingDamage,
    ],
    [
      "Unknown direction hits",
      formatDecimalString(quality.unknownDirectionHits),
      quality.unknownDirectionDamage,
    ],
    [
      "Unknown characters / hits",
      quality.unknownCharacterCount,
      formatDecimalString(quality.unknownCharacterHits),
    ],
    [
      "Unmapped skills / hits",
      quality.unmappedSkillRows,
      formatDecimalString(quality.unmappedSkillHits),
    ],
    ["Unmapped gameplay effects", quality.unmappedGameplayEffectCount, "—"],
    [
      "Time stop events / intervals",
      formatDecimalString(quality.timeStopEventCount),
      quality.timeStopIntervalCount,
    ],
    [
      "Abyss events / server corrections",
      formatDecimalString(quality.abyssEventCount),
      formatDecimalString(quality.serverDamageCorrections),
    ],
  ] as const;
  return (
    <section aria-labelledby="diagnostics-quality" className="pb-8">
      <SectionHeading
        icon={Gauge}
        id="diagnostics-quality"
        title="Parse Quality"
        description="Aggregated counters from live capture or replay"
      >
        <Badge variant="outline">{t(SOURCE_LABELS[quality.source])}</Badge>
      </SectionHeading>
      <dl className="grid border-y min-[720px]:grid-cols-2 min-[1260px]:grid-cols-3">
        {rows.map(([label, primary, secondary]) => (
          <div
            className="flex items-center justify-between gap-4 border-b px-4 py-3 min-[720px]:border-r"
            key={label}
          >
            <dt className="min-w-0 text-xs text-muted-foreground">
              {t(label)}
            </dt>
            <dd className="shrink-0 font-mono text-sm font-medium tabular-nums">
              {typeof primary === "number" ? primary.toLocaleString() : primary}
              <span className="px-1.5 text-muted-foreground">/</span>
              {typeof secondary === "number"
                ? secondary.toLocaleString()
                : secondary}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

function SectionHeading({
  icon: Icon,
  id,
  title,
  description,
  children,
}: {
  icon: typeof Activity;
  id: string;
  title: string;
  description: string;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
      <div className="flex min-w-0 items-start gap-2.5">
        <Icon
          className="mt-0.5 size-4 text-muted-foreground"
          aria-hidden="true"
        />
        <div className="min-w-0">
          <h2 id={id} className="text-sm font-semibold">
            {t(title)}
          </h2>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t(description)}
          </p>
        </div>
      </div>
      {children}
    </div>
  );
}

function translateDiagnosticToken(key: string): string {
  if (key in CAPTURE_PHASE_LABELS) {
    return t(CAPTURE_PHASE_LABELS[key as keyof typeof CAPTURE_PHASE_LABELS]);
  }
  if (key in SOURCE_LABELS) {
    return t(SOURCE_LABELS[key as keyof typeof SOURCE_LABELS]);
  }
  if (key in STATUS_LABELS) {
    return t(STATUS_LABELS[key as keyof typeof STATUS_LABELS]);
  }
  return t(key);
}

function DiagnosticsLoading() {
  return (
    <div className="space-y-4 p-4">
      <Skeleton className="h-24 w-full" />
      <Skeleton className="h-12 w-full" />
      <Skeleton className="h-64 w-full" />
      <Skeleton className="h-36 w-full" />
    </div>
  );
}

function DiagnosticsLoadError({
  error,
  onRetry,
}: {
  error: DiagnosticsCommandError;
  onRetry: () => void;
}) {
  return (
    <Empty className="min-h-full border-0">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <TriangleAlert aria-hidden="true" />
        </EmptyMedia>
        <EmptyTitle>{t("Diagnostics could not be loaded")}</EmptyTitle>
        <EmptyDescription>
          {tf(error.messageKey, error.messageArguments)}
        </EmptyDescription>
      </EmptyHeader>
      <Button onClick={onRetry}>
        <RefreshCw aria-hidden="true" />
        {t("Retry")}
      </Button>
    </Empty>
  );
}

function DiagnosticsNotice({
  error,
  onDismiss,
}: {
  error: DiagnosticsCommandError;
  onDismiss: () => void;
}) {
  return createPortal(
    <div className="pointer-events-none fixed inset-x-0 top-14 z-[90] flex justify-center px-4">
      <Alert
        variant="destructive"
        className="pointer-events-auto w-full max-w-xl shadow-xl"
      >
        <Activity className="size-4" aria-hidden="true" />
        <AlertTitle>{t("Diagnostics operation failed")}</AlertTitle>
        <AlertDescription>
          {tf(error.messageKey, error.messageArguments)}
        </AlertDescription>
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
