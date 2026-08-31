import { useEffect, useMemo, useState } from "react";
import {
  ArrowDownRight,
  ArrowUpRight,
  Download,
  Eye,
  GitCompareArrows,
  History as HistoryIcon,
  Minus,
  Radio,
  RefreshCw,
  Save,
  Trash2,
  Upload,
} from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import {
  ContextMenu,
  ContextMenuItem,
  ContextMenuPopup,
  ContextMenuPortal,
  ContextMenuPositioner,
  ContextMenuTrigger,
} from "@/components/ui/menu";
import { Skeleton } from "@/components/ui/skeleton";
import { useCharacterAvatar } from "@/hooks/use-character-avatar";
import { t, tf } from "@/lib/i18n";
import type {
  HistoryCharacter,
  HistoryComparison,
  HistoryHalf,
  HistoryRecord,
  HistorySkill,
} from "@/lib/tauri/history-contract";
import { cn } from "@/lib/utils";
import { technicalClient } from "@/lib/tauri/technical-client";
import { parseTechnicalCommandError } from "@/lib/tauri/technical-contract";
import { diagnosticsClient } from "@/lib/tauri/diagnostics-client";
import { diagnosticsError } from "@/lib/tauri/diagnostics-contract";

import {
  adjacentHistoryRecordId,
  historyComparisonWarningKeys,
  historyDeltaTone,
  historyDurationFormat,
  historyRecordById,
  nextHistoryRecordIndex,
  selectHistoryRecord,
  validHistoryComparisonPair,
} from "./history-view-model";
import { useHistory } from "./use-history";

export function HistoryPage() {
  const history = useHistory();
  const {
    clearComparison,
    clearPreferredRecordId,
    compare,
    preferredRecordId,
  } = history;
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [leftId, setLeftId] = useState("");
  const [rightId, setRightId] = useState("");
  const [capturePending, setCapturePending] = useState(false);
  const [captureError, setCaptureError] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<string | null>(null);

  const records = useMemo(
    () =>
      history.state.status === "ready" ? history.state.snapshot.records : [],
    [history.state],
  );
  const historyRevision =
    history.state.status === "ready"
      ? history.state.snapshot.revision
      : "loading";
  useEffect(() => {
    setSelectedId((current) => selectHistoryRecord(records, current));
    setLeftId((current) => selectHistoryRecord(records, current) ?? "");
    setRightId((current) => {
      if (records.some((record) => record.id === current)) return current;
      return records[1]?.id ?? "";
    });
  }, [records]);

  useEffect(() => {
    if (
      preferredRecordId !== null &&
      records.some((record) => record.id === preferredRecordId)
    ) {
      setSelectedId(preferredRecordId);
      clearPreferredRecordId();
    }
  }, [clearPreferredRecordId, preferredRecordId, records]);

  useEffect(() => {
    if (!validHistoryComparisonPair(records, leftId, rightId)) {
      clearComparison();
      return;
    }
    void compare(leftId, rightId);
  }, [clearComparison, compare, historyRevision, leftId, records, rightId]);

  useEffect(() => {
    if (contextMenu === null) return;
    const closeContextMenuOnWindowBlur = () => setContextMenu(null);
    window.addEventListener("blur", closeContextMenuOnWindowBlur);
    return () => {
      window.removeEventListener("blur", closeContextMenuOnWindowBlur);
    };
  }, [contextMenu]);

  const selected = historyRecordById(records, selectedId);
  const busy = history.pendingAction !== null;

  async function startCapture() {
    if (capturePending) return;
    setCapturePending(true);
    setCaptureError(null);
    try {
      await technicalClient.startCapture();
    } catch (error) {
      setCaptureError(parseTechnicalCommandError(error).messageKey);
    } finally {
      setCapturePending(false);
    }
  }

  async function importCaptureJson() {
    if (capturePending) return;
    setCapturePending(true);
    setCaptureError(null);
    try {
      await diagnosticsClient.importJson();
    } catch (error) {
      setCaptureError(diagnosticsError(error).messageKey);
    } finally {
      setCapturePending(false);
    }
  }

  if (history.state.status === "loading") return <HistoryLoading />;
  if (history.state.status === "error") {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center p-6">
        <Alert variant="destructive" className="max-w-xl">
          <AlertTitle>{t("History records could not be loaded.")}</AlertTitle>
          <AlertDescription>{t(history.state.messageKey)}</AlertDescription>
          <Button
            className="mt-3"
            size="sm"
            variant="outline"
            onClick={() => void history.refresh()}
          >
            {t("Retry")}
          </Button>
        </Alert>
      </div>
    );
  }

  return (
    <ContextMenu
      open={contextMenu !== null}
      onOpenChange={(open) => {
        if (!open) setContextMenu(null);
      }}
    >
      <ContextMenuTrigger
        render={
          <section className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
            <header className="flex flex-wrap items-center gap-2 border-b px-3 py-2 min-[640px]:px-4">
              <Button
                disabled={busy}
                size="sm"
                title={t(
                  "Save a de-identified stats summary; no packets, payload, IP, port or local paths",
                )}
                onClick={() => void history.saveCurrent()}
              >
                <Save className="size-4" /> {t("Save This Summary")}
              </Button>
              <Button
                disabled={busy}
                size="sm"
                variant="outline"
                title={t("Import a previously exported history record")}
                onClick={() => void history.importFile()}
              >
                <Upload className="size-4" /> {t("Import record JSON")}
              </Button>
              <Button
                aria-label={t("Reload")}
                disabled={busy}
                size="icon-sm"
                variant="outline"
                onClick={() => void history.refresh()}
              >
                <RefreshCw
                  className={cn(
                    "size-4",
                    history.pendingAction === "reload" && "animate-spin",
                  )}
                />
              </Button>
              <span className="text-xs text-muted-foreground">
                {tf("{} records", [String(records.length)])}
              </span>
              {history.state.snapshot.skippedFiles > 0 && (
                <span className="text-xs text-amber-600 dark:text-amber-300">
                  {tf("Skipped {} corrupt files", [
                    String(history.state.snapshot.skippedFiles),
                  ])}
                </span>
              )}
              {history.statusMessageKey && (
                <span
                  className="text-xs text-muted-foreground"
                  aria-live="polite"
                >
                  {t(history.statusMessageKey)}
                </span>
              )}
            </header>

            {records.length > 0 && (
              <p className="border-b px-4 py-1.5 text-[11px] text-muted-foreground">
                {t(
                  "Click a record for details; right-click to compare, export or delete",
                )}
              </p>
            )}

            {history.mutationError && (
              <Alert variant="destructive" className="mx-5 mt-4 w-auto">
                <AlertTitle>{t("History operation failed")}</AlertTitle>
                <AlertDescription>
                  {t(history.mutationError.messageKey)}
                </AlertDescription>
              </Alert>
            )}

            {captureError && (
              <Alert variant="destructive" className="mx-5 mt-4 w-auto">
                <AlertTitle>{t("Capture start failed")}</AlertTitle>
                <AlertDescription>{t(captureError)}</AlertDescription>
              </Alert>
            )}

            {history.undoDeletion && (
              <Alert className="mx-5 mt-4 w-auto">
                <AlertTitle>{t("History record deleted")}</AlertTitle>
                <AlertDescription className="mt-2 flex flex-wrap items-center justify-between gap-2">
                  <span>{t("You can restore it for a few seconds.")}</span>
                  <Button
                    disabled={busy}
                    size="sm"
                    variant="outline"
                    onClick={() => void history.restoreDeleted()}
                  >
                    {t("Undo")}
                  </Button>
                </AlertDescription>
              </Alert>
            )}

            {records.length === 0 ? (
              <Empty className="m-5 min-h-72 border">
                <EmptyHeader>
                  <EmptyMedia variant="icon">
                    <HistoryIcon />
                  </EmptyMedia>
                  <EmptyTitle>{t("No history records yet")}</EmptyTitle>
                  <EmptyDescription>
                    {t(
                      "Save the current combat summary or import a history record JSON file.",
                    )}
                  </EmptyDescription>
                </EmptyHeader>
                <div className="flex flex-wrap justify-center gap-2">
                  <Button
                    disabled={capturePending}
                    onClick={() => void startCapture()}
                  >
                    <Radio className="size-4" /> {t("Start Capture")}
                  </Button>
                  <Button
                    disabled={busy}
                    onClick={() => void history.saveCurrent()}
                  >
                    <Save className="size-4" /> {t("Save This Summary")}
                  </Button>
                  <Button
                    disabled={capturePending}
                    variant="outline"
                    onClick={() => void importCaptureJson()}
                  >
                    <Upload className="size-4" /> {t("Import Capture JSON")}
                  </Button>
                  <Button
                    disabled={busy}
                    variant="outline"
                    onClick={() => void history.importFile()}
                  >
                    <Upload className="size-4" /> {t("Import record JSON")}
                  </Button>
                </div>
              </Empty>
            ) : (
              <div className="grid min-h-0 flex-1 grid-cols-1 items-start gap-3 overflow-y-auto p-3 min-[900px]:grid-cols-[300px_minmax(0,1fr)]">
                <div
                  className="flex min-w-0 gap-2 overflow-x-auto rounded-lg border bg-background p-2 min-[900px]:sticky min-[900px]:top-0 min-[900px]:block min-[900px]:max-h-[calc(100vh-7rem)] min-[900px]:overflow-y-auto"
                  role="listbox"
                  aria-label={t("History records")}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      document
                        .querySelector<HTMLElement>("[data-history-detail]")
                        ?.focus();
                      return;
                    }
                    if (event.key !== "ArrowDown" && event.key !== "ArrowUp")
                      return;
                    const buttons = Array.from(
                      event.currentTarget.querySelectorAll<HTMLButtonElement>(
                        "[data-history-record]",
                      ),
                    );
                    const current = buttons.indexOf(
                      event.target as HTMLButtonElement,
                    );
                    if (current < 0) return;
                    event.preventDefault();
                    const next = nextHistoryRecordIndex(
                      buttons.length,
                      current,
                      event.key === "ArrowDown" ? 1 : -1,
                    );
                    buttons[next]?.focus();
                    setSelectedId(records[next]?.id ?? null);
                  }}
                >
                  {records.map((record) => (
                    <button
                      key={record.id}
                      type="button"
                      role="option"
                      data-history-record
                      aria-selected={record.id === selectedId}
                      className={cn(
                        "flex min-h-16 min-w-64 shrink-0 flex-col gap-0.5 rounded-md px-3 py-2 text-left text-sm hover:bg-muted min-[900px]:mb-1 min-[900px]:w-full min-[900px]:min-w-0",
                        record.id === selectedId &&
                          "bg-primary text-primary-foreground hover:bg-primary",
                      )}
                      onClick={() => setSelectedId(record.id)}
                      onContextMenu={() => {
                        setSelectedId(record.id);
                        setContextMenu(record.id);
                      }}
                    >
                      <span className="font-medium">{record.displayTime}</span>
                      {record.partyLabel && (
                        <span
                          className={cn(
                            "truncate text-xs text-muted-foreground",
                            record.id === selectedId &&
                              "text-primary-foreground/75",
                          )}
                          title={record.partyLabel}
                        >
                          {record.partyLabel}
                        </span>
                      )}
                      <span
                        className={cn(
                          "text-xs text-muted-foreground",
                          record.id === selectedId &&
                            "text-primary-foreground/75",
                        )}
                      >
                        {number(record.summary.totalDps)} DPS ·{" "}
                        {number(record.summary.totalDamage)}
                      </span>
                    </button>
                  ))}
                </div>

                <div className="@container/history min-h-0 min-w-0 space-y-4">
                  {selected && (
                    <div
                      data-history-detail
                      tabIndex={-1}
                      className="space-y-4 outline-none"
                    >
                      <RecordDetail
                        busy={busy}
                        record={selected}
                        onDelete={() => {
                          void history.deleteRecord(selected.id);
                        }}
                        onExport={() =>
                          void history.exportRecordFile(selected.id)
                        }
                        onPrediction={(line) =>
                          void history.setPrediction(selected.id, line)
                        }
                      />
                    </div>
                  )}
                  <ComparePanel
                    busy={busy}
                    comparison={history.comparison}
                    leftId={leftId}
                    records={records}
                    rightId={rightId}
                    onLeftChange={setLeftId}
                    onRightChange={setRightId}
                    onUseAdjacent={() => {
                      if (!selected) return;
                      const adjacent = adjacentHistoryRecordId(
                        records,
                        selected.id,
                      );
                      if (adjacent) {
                        setLeftId(selected.id);
                        setRightId(adjacent);
                      }
                    }}
                  />
                </div>
              </div>
            )}
          </section>
        }
      />
      {contextMenu !== null && (
        <HistoryContextMenu
          busy={busy}
          recordId={contextMenu}
          records={records}
          onDelete={(recordId) => void history.deleteRecord(recordId)}
          onExport={(recordId) => void history.exportRecordFile(recordId)}
          onSelect={setSelectedId}
          onCompare={(recordId, adjacentId) => {
            setSelectedId(recordId);
            setLeftId(recordId);
            setRightId(adjacentId);
          }}
        />
      )}
    </ContextMenu>
  );
}

function HistoryContextMenu({
  busy,
  recordId,
  records,
  onCompare,
  onDelete,
  onExport,
  onSelect,
}: {
  busy: boolean;
  recordId: string;
  records: HistoryRecord[];
  onCompare: (recordId: string, adjacentId: string) => void;
  onDelete: (recordId: string) => void;
  onExport: (recordId: string) => void;
  onSelect: (recordId: string) => void;
}) {
  const adjacent = adjacentHistoryRecordId(records, recordId);
  return (
    <ContextMenuPortal>
      <ContextMenuPositioner>
        <ContextMenuPopup
          aria-label={t("History records")}
          className="flex w-52 flex-col gap-0.5 p-1.5"
        >
          <HistoryMenuButton
            icon={<Eye />}
            label="View details"
            onClick={() => onSelect(recordId)}
          />
          <HistoryMenuButton
            disabled={busy || adjacent === null}
            icon={<GitCompareArrows />}
            label="Compare with adjacent record"
            onClick={() => {
              if (adjacent !== null) onCompare(recordId, adjacent);
            }}
          />
          <HistoryMenuButton
            disabled={busy}
            icon={<Download />}
            label="Export record JSON"
            onClick={() => onExport(recordId)}
          />
          <HistoryMenuButton
            danger
            disabled={busy}
            icon={<Trash2 />}
            label="Delete"
            onClick={() => onDelete(recordId)}
          />
        </ContextMenuPopup>
      </ContextMenuPositioner>
    </ContextMenuPortal>
  );
}

function HistoryMenuButton({
  danger = false,
  disabled = false,
  icon,
  label,
  onClick,
}: {
  danger?: boolean;
  disabled?: boolean;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <ContextMenuItem
      className={cn("w-full gap-2", danger && "text-destructive")}
      disabled={disabled}
      onClick={onClick}
    >
      <span className="[&>svg]:size-4" aria-hidden="true">
        {icon}
      </span>
      {t(label)}
    </ContextMenuItem>
  );
}

function RecordDetail({
  busy,
  record,
  onDelete,
  onExport,
  onPrediction,
}: {
  busy: boolean;
  record: HistoryRecord;
  onDelete: () => void;
  onExport: () => void;
  onPrediction: (line: "upper" | "lower") => void;
}) {
  const summary = record.summary;
  return (
    <>
      <Card size="sm">
        <CardHeader>
          <CardTitle>{record.displayTime}</CardTitle>
          <CardDescription>
            {t(
              summary.dpsTimeBasis === "subtract_time_stop"
                ? "Exclude Time Stop"
                : "Real Time (incl. time stop)",
            )}
            {` · ${t(summary.reactionDamageSeparated ? "Reactions separated from character damage" : "Reactions included in character damage")}`}
          </CardDescription>
          <CardAction className="flex gap-1">
            {record.canSetLowerPrediction && (
              <Button
                disabled={busy}
                size="sm"
                variant="outline"
                onClick={() => onPrediction("lower")}
              >
                {t("Set as Lower Prediction")}
              </Button>
            )}
            {record.canSetUpperPrediction && (
              <Button
                disabled={busy}
                size="sm"
                variant="outline"
                onClick={() => onPrediction("upper")}
              >
                {t("Set as Upper Prediction")}
              </Button>
            )}
            <Button
              aria-label={t("Export JSON")}
              disabled={busy}
              size="icon-sm"
              variant="ghost"
              onClick={onExport}
            >
              <Download />
            </Button>
            <Button
              aria-label={t("Delete")}
              disabled={busy}
              size="icon-sm"
              variant="ghost"
              onClick={onDelete}
            >
              <Trash2 />
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent>
          <div className="grid grid-cols-2 gap-2 @min-[760px]/history:grid-cols-4">
            <Metric label="Total DPS" value={number(summary.totalDps)} />
            <Metric label="Total Damage" value={number(summary.totalDamage)} />
            <Metric
              label="Combat Time"
              value={formatHistoryDuration(summary.durationSeconds)}
            />
            <Metric
              label="Parse Quality"
              value={tf("{} hits / {} pending", [
                summary.quality.hitCount,
                summary.quality.unmappedSkillHits,
              ])}
            />
          </div>
          <div className="mt-3 flex flex-wrap gap-2 text-xs text-muted-foreground">
            <Badge variant="outline">{t(summary.quality.source)}</Badge>
            <span>
              {t("Packets")}: {summary.quality.packetCount}
            </span>
            <span>
              {t("Unmapped skill hits")}: {summary.quality.unmappedSkillHits}
            </span>
            <span>
              {t("Unknown character hits")}:{" "}
              {summary.quality.unknownCharacterHits}
            </span>
          </div>
        </CardContent>
      </Card>
      {summary.abyss.firstHalf !== null || summary.abyss.secondHalf !== null ? (
        <div className="grid grid-cols-1 gap-4 @min-[980px]/history:grid-cols-2">
          {summary.abyss.firstHalf && (
            <HalfCard half={summary.abyss.firstHalf} />
          )}
          {summary.abyss.secondHalf && (
            <HalfCard half={summary.abyss.secondHalf} />
          )}
        </div>
      ) : (
        <Breakdown
          characters={summary.characters}
          hiddenCharacterCount={summary.hiddenCharacterCount}
          hiddenSkillCount={summary.hiddenSkillCount}
          skills={summary.skills}
        />
      )}
    </>
  );
}

function HalfCard({ half }: { half: HistoryHalf }) {
  return (
    <Card className="@container/half">
      <CardHeader>
        <CardTitle>
          {t(half.half === "first" ? "First Half" : "Second Half")}
        </CardTitle>
        <CardDescription>
          {number(half.totalDps)} DPS · {number(half.totalDamage)} {t("Damage")}{" "}
          · {formatHistoryDuration(half.durationSeconds)}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <Breakdown
          characters={half.characters}
          hiddenCharacterCount={half.hiddenCharacterCount}
          hiddenSkillCount={half.hiddenSkillCount}
          skills={half.skills}
          characterTitle="Character Contribution"
          skillTitle="Skill Composition"
          embedded
        />
      </CardContent>
    </Card>
  );
}

function Breakdown({
  characters,
  hiddenCharacterCount = 0,
  hiddenSkillCount = 0,
  skills,
  embedded = false,
  characterTitle = "Character",
  skillTitle = "Skill",
}: {
  characters: HistoryCharacter[];
  hiddenCharacterCount?: number;
  hiddenSkillCount?: number;
  skills: HistorySkill[];
  embedded?: boolean;
  characterTitle?: string;
  skillTitle?: string;
}) {
  const content = (
    <div
      className={cn(
        "grid grid-cols-1 gap-4",
        embedded
          ? "@min-[700px]/half:grid-cols-2"
          : "@min-[720px]/history:grid-cols-2",
      )}
    >
      <CharacterRows title={characterTitle} rows={characters} />
      <div className="min-w-0">
        <Rows
          title={skillTitle}
          rows={skills.map((row, index) => ({
            key: `${row.charId}:${row.name}:${index}`,
            title: row.name,
            value: number(row.damage),
            detail: `${row.charName} · ${row.hits} ${t("hits")} · ${row.damageSharePercent.toFixed(1)}%`,
            badge: row.isFollowUp ? t("Follow-up") : undefined,
          }))}
        />
        {(hiddenCharacterCount > 0 || hiddenSkillCount > 0) && (
          <p className="mt-2 text-xs text-muted-foreground">
            {t("More rows are available in the full record")}: +
            {hiddenCharacterCount} {t("Characters")}, +{hiddenSkillCount}{" "}
            {t("Skills")}
          </p>
        )}
      </div>
    </div>
  );
  return embedded ? (
    content
  ) : (
    <Card>
      <CardContent>{content}</CardContent>
    </Card>
  );
}

function CharacterRows({
  title,
  rows,
}: {
  title: string;
  rows: HistoryCharacter[];
}) {
  return (
    <div className="min-w-0">
      <h3 className="mb-2 text-xs font-semibold uppercase text-muted-foreground">
        {t(title)}
      </h3>
      <div className="flex flex-col gap-1.5">
        {rows.length === 0 ? (
          <p className="text-xs text-muted-foreground">{t("No data")}</p>
        ) : (
          rows.map((row) => (
            <div
              key={row.charId}
              className="grid min-w-0 grid-cols-[2.5rem_minmax(0,1fr)] items-center gap-2.5 rounded-lg bg-muted/55 p-2 @min-[430px]/history:grid-cols-[2.5rem_minmax(0,1fr)_auto]"
            >
              <CharacterAvatar character={row} />
              <div className="min-w-0">
                <p className="truncate text-sm font-medium" title={row.name}>
                  {row.name}
                </p>
                <p className="text-xs text-muted-foreground">
                  {number(row.damage)} · {row.damageSharePercent.toFixed(1)}%
                </p>
              </div>
              <span className="col-start-2 whitespace-nowrap font-mono text-xs font-medium @min-[430px]/history:col-start-auto">
                {number(row.dps)} DPS
              </span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

function CharacterAvatar({ character }: { character: HistoryCharacter }) {
  const avatarUrl = useCharacterAvatar(character.charId);
  const initial = character.name.trim().charAt(0) || "?";
  return (
    <div className="flex size-10 shrink-0 items-center justify-center overflow-hidden rounded-lg border bg-primary/10 text-sm font-semibold text-primary">
      {avatarUrl ? (
        <img
          alt=""
          className="size-full object-cover"
          decoding="async"
          draggable={false}
          loading="lazy"
          src={avatarUrl}
        />
      ) : (
        <span aria-hidden="true">{initial}</span>
      )}
    </div>
  );
}

function Rows({
  title,
  rows,
}: {
  title: string;
  rows: Array<{
    key: string;
    title: string;
    value: string;
    detail: string;
    badge?: string;
  }>;
}) {
  return (
    <div className="min-w-0">
      <h3 className="mb-2 text-xs font-semibold uppercase text-muted-foreground">
        {t(title)}
      </h3>
      <div className="flex flex-col gap-1.5">
        {rows.length === 0 ? (
          <p className="text-xs text-muted-foreground">{t("No data")}</p>
        ) : (
          rows.map((row) => (
            <div
              key={row.key}
              className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-lg bg-muted/55 px-2.5 py-2"
            >
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-1.5">
                  <p
                    className="break-words text-sm leading-snug font-medium"
                    title={row.title}
                  >
                    {row.title}
                  </p>
                  {row.badge && <Badge variant="secondary">{row.badge}</Badge>}
                </div>
                <p className="mt-0.5 break-words text-xs text-muted-foreground">
                  {row.detail}
                </p>
              </div>
              <span className="whitespace-nowrap font-mono text-xs">
                {row.value}
              </span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

function ComparePanel({
  busy,
  comparison,
  leftId,
  records,
  rightId,
  onLeftChange,
  onRightChange,
  onUseAdjacent,
}: {
  busy: boolean;
  comparison: HistoryComparison | null;
  leftId: string;
  records: HistoryRecord[];
  rightId: string;
  onLeftChange: (value: string) => void;
  onRightChange: (value: string) => void;
  onUseAdjacent: () => void;
}) {
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>{t("Compare")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid grid-cols-1 gap-2 @min-[720px]/history:grid-cols-2">
          <HistorySelect
            value={leftId}
            records={records}
            onChange={onLeftChange}
            label="Baseline"
          />
          <HistorySelect
            value={rightId}
            records={records}
            onChange={onRightChange}
            label="Compare"
          />
        </div>
        <Button
          disabled={busy || records.length < 2}
          size="sm"
          variant="ghost"
          onClick={onUseAdjacent}
        >
          {t("Compare selected with adjacent")}
        </Button>
        {comparison ? (
          <ComparisonResult comparison={comparison} />
        ) : (
          <p className="text-sm text-muted-foreground">
            {t("Select two different records")}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function HistorySelect({
  value,
  records,
  onChange,
  label,
}: {
  value: string;
  records: HistoryRecord[];
  onChange: (value: string) => void;
  label: string;
}) {
  return (
    <label className="flex flex-col gap-1 text-xs text-muted-foreground">
      {t(label)}
      <select
        className="h-9 rounded-md border bg-background px-2 text-sm text-foreground"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        <option value="">{t("Select a record")}</option>
        {records.map((record) => (
          <option key={record.id} value={record.id}>
            {record.partyLabel
              ? `${record.displayTime} · ${record.partyLabel}`
              : record.displayTime}
          </option>
        ))}
      </select>
    </label>
  );
}

function ComparisonResult({ comparison }: { comparison: HistoryComparison }) {
  const warnings = historyComparisonWarningKeys(comparison);
  return (
    <div className="space-y-3 border-t pt-3">
      {warnings.map((warning) => (
        <Alert key={warning}>
          <AlertDescription>{t(warning)}</AlertDescription>
        </Alert>
      ))}
      <div className="grid grid-cols-1 gap-2 @min-[520px]/history:grid-cols-3">
        <Delta label="Total DPS Δ" value={comparison.totalDpsDelta} />
        <Delta label="Total Damage Δ" value={comparison.totalDamageDelta} />
        <Delta label="Time Δ" value={comparison.durationDelta} />
      </div>
      <Rows
        title="Character Δ"
        rows={comparison.characterDeltas.map((row) => ({
          key: String(row.charId),
          title: row.name,
          value: signed(row.deltaDps),
          detail: `${number(row.leftDps)} → ${number(row.rightDps)} DPS`,
        }))}
      />
      <Rows
        title="Skill Δ"
        rows={comparison.skillDeltas.map((row, index) => ({
          key: `${row.name}:${row.category}:${index}`,
          title: row.name,
          value: signed(row.deltaDamage),
          detail: `${number(row.leftDamage)} → ${number(row.rightDamage)}`,
        }))}
      />
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg bg-muted/60 p-3">
      <p className="text-xs text-muted-foreground">{t(label)}</p>
      <p className="mt-1 font-mono text-lg font-semibold">{value}</p>
    </div>
  );
}

function Delta({ label, value }: { label: string; value: number }) {
  const tone = historyDeltaTone(value);
  return (
    <div className="rounded-lg bg-muted/60 p-3">
      <p className="text-xs text-muted-foreground">{t(label)}</p>
      <p
        className={cn(
          "mt-1 flex items-center gap-1 font-mono font-semibold",
          tone === "positive" && "text-emerald-600 dark:text-emerald-300",
          tone === "negative" && "text-destructive",
          tone === "neutral" && "text-foreground",
        )}
      >
        {tone === "positive" ? (
          <ArrowUpRight className="size-4" />
        ) : tone === "negative" ? (
          <ArrowDownRight className="size-4" />
        ) : (
          <Minus className="size-4" />
        )}
        {signed(value)}
      </p>
    </div>
  );
}

function HistoryLoading() {
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 p-5">
      <Skeleton className="h-12 w-full" />
      <div className="grid flex-1 grid-cols-1 gap-3 min-[900px]:grid-cols-[300px_1fr]">
        <Skeleton className="h-24 min-[900px]:h-full" />
        <Skeleton className="min-h-72" />
      </div>
    </div>
  );
}

function number(value: number): string {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(
    value,
  );
}

function signed(value: number): string {
  if (value > 0) return `+${number(value)}`;
  if (value < 0) return `-${number(Math.abs(value))}`;
  return number(0);
}

function formatHistoryDuration(seconds: number): string {
  const format = historyDurationFormat(seconds);
  return tf(format.key, format.arguments);
}
