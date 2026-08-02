import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDownRight,
  ArrowUpRight,
  Download,
  History as HistoryIcon,
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
import { Skeleton } from "@/components/ui/skeleton";
import { t } from "@/lib/i18n";
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

import { historyCharacterAvatarUrl } from "./history-character-avatar";
import {
  adjacentHistoryRecordId,
  historyRecordById,
  nextHistoryRecordIndex,
  selectHistoryRecord,
} from "./history-view-model";
import { useHistory } from "./use-history";

export function HistoryPage() {
  const history = useHistory();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [leftId, setLeftId] = useState("");
  const [rightId, setRightId] = useState("");
  const [capturePending, setCapturePending] = useState(false);
  const [captureError, setCaptureError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const records = useMemo(
    () =>
      history.state.status === "ready" ? history.state.snapshot.records : [],
    [history.state],
  );
  useEffect(() => {
    setSelectedId((current) => selectHistoryRecord(records, current));
    setLeftId((current) => selectHistoryRecord(records, current) ?? "");
    setRightId((current) => {
      if (records.some((record) => record.id === current)) return current;
      return records[1]?.id ?? "";
    });
  }, [records]);

  const selected = historyRecordById(records, selectedId);
  const busy = history.pendingAction !== null;

  async function importFile(file: File | undefined) {
    if (file === undefined || history.state.status !== "ready") return;
    if (BigInt(file.size) > BigInt(history.state.snapshot.maxImportBytes)) {
      window.alert(t("History record exceeds the supported import size."));
      return;
    }
    await history.importJson(await file.text());
    if (inputRef.current) inputRef.current.value = "";
  }

  async function exportRecord(recordId: string) {
    const result = await history.exportRecord(recordId);
    if (result === null) return;
    const url = URL.createObjectURL(
      new Blob([result.json], { type: "application/json" }),
    );
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = result.fileName;
    anchor.click();
    URL.revokeObjectURL(url);
  }

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
    <section className="flex min-h-0 min-w-0 flex-1 flex-col bg-muted/20">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b bg-background px-3 py-3 min-[640px]:px-5">
        <div>
          <h1 className="text-base font-semibold">{t("History")}</h1>
          <p className="text-xs text-muted-foreground">
            {records.length} {t("records")}
            {history.state.snapshot.skippedFiles > 0
              ? ` · ${history.state.snapshot.skippedFiles} ${t("corrupt files skipped")}`
              : ""}
          </p>
        </div>
        <div className="flex w-full flex-wrap items-center gap-2 min-[640px]:w-auto">
          <Button
            disabled={busy}
            size="sm"
            onClick={() => void history.saveCurrent()}
          >
            <Save className="size-4" /> {t("Save This Summary")}
          </Button>
          <input
            ref={inputRef}
            className="hidden"
            type="file"
            accept="application/json,.json"
            onChange={(event) => void importFile(event.target.files?.[0])}
          />
          <Button
            disabled={busy}
            size="sm"
            variant="outline"
            onClick={() => inputRef.current?.click()}
          >
            <Upload className="size-4" /> {t("Import Record JSON")}
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
        </div>
      </header>

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
            <Button disabled={busy} onClick={() => void history.saveCurrent()}>
              <Save className="size-4" /> {t("Save This Summary")}
            </Button>
            <Button
              disabled
              title={t(
                "Capture JSON replay is pending its detail-window migration.",
              )}
              variant="outline"
            >
              <Upload className="size-4" /> {t("Import Capture JSON")}
            </Button>
            <Button
              disabled={busy}
              variant="outline"
              onClick={() => inputRef.current?.click()}
            >
              <Upload className="size-4" /> {t("Import Record JSON")}
            </Button>
          </div>
        </Empty>
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-1 items-start gap-4 overflow-y-auto p-3 min-[1280px]:grid-cols-[17rem_minmax(0,1fr)] min-[1280px]:p-4">
          <div
            className="flex min-w-0 gap-2 overflow-x-auto rounded-xl border bg-background p-2 min-[1280px]:sticky min-[1280px]:top-0 min-[1280px]:block min-[1280px]:max-h-[calc(100vh-7rem)] min-[1280px]:overflow-y-auto"
            role="listbox"
            aria-label={t("History records")}
            onKeyDown={(event) => {
              if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
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
                  "flex min-w-64 shrink-0 flex-col gap-1 rounded-lg px-3 py-2 text-left text-sm hover:bg-muted min-[1280px]:mb-1 min-[1280px]:w-full min-[1280px]:min-w-0",
                  record.id === selectedId &&
                    "bg-primary text-primary-foreground hover:bg-primary",
                )}
                onClick={() => setSelectedId(record.id)}
              >
                <span className="font-medium">{record.displayTime}</span>
                <span
                  className={cn(
                    "text-xs text-muted-foreground",
                    record.id === selectedId && "text-primary-foreground/75",
                  )}
                >
                  {number(record.summary.totalDps)} DPS ·{" "}
                  {number(record.summary.totalDamage)} {t("Damage")}
                </span>
                {record.partyLabel && (
                  <span
                    className={cn(
                      "truncate text-xs text-muted-foreground",
                      record.id === selectedId && "text-primary-foreground/75",
                    )}
                    title={record.partyLabel}
                  >
                    {record.partyLabel}
                  </span>
                )}
              </button>
            ))}
          </div>

          <div className="@container/history min-h-0 min-w-0 space-y-4">
            {selected && (
              <RecordDetail
                busy={busy}
                record={selected}
                onDelete={() => {
                  if (window.confirm(t("Delete this history record?")))
                    void history.deleteRecord(selected.id);
                }}
                onExport={() => void exportRecord(selected.id)}
                onPrediction={(line) =>
                  void history.setPrediction(selected.id, line)
                }
              />
            )}
            <ComparePanel
              busy={busy}
              comparison={history.comparison}
              leftId={leftId}
              records={records}
              rightId={rightId}
              onCompare={() => void history.compare(leftId, rightId)}
              onLeftChange={setLeftId}
              onRightChange={setRightId}
              onUseAdjacent={() => {
                if (!selected) return;
                const adjacent = adjacentHistoryRecordId(records, selected.id);
                if (adjacent) {
                  setLeftId(selected.id);
                  setRightId(adjacent);
                  void history.compare(selected.id, adjacent);
                }
              }}
            />
          </div>
        </div>
      )}
    </section>
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
      <Card>
        <CardHeader>
          <CardTitle>{record.displayTime}</CardTitle>
          <CardDescription>
            {t(
              summary.dpsTimeBasis === "subtract_time_stop"
                ? "Exclude Time Stop"
                : "Real Time",
            )}
            {` · ${t(summary.reactionDamageSeparated ? "Reaction Damage Separated" : "Reaction Damage Combined")}`}
          </CardDescription>
          <CardAction className="flex gap-1">
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
              value={`${summary.durationSeconds.toFixed(1)}s`}
            />
            <Metric
              label="Parse Quality"
              value={`${summary.quality.hitCount} ${t("hits")}`}
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
      {summary.abyss.detected ? (
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
      <Card size="sm">
        <CardHeader>
          <CardTitle>{t("Abyss Prediction")}</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-2">
          <Button
            disabled={busy || !record.canSetUpperPrediction}
            size="sm"
            variant="outline"
            onClick={() => onPrediction("upper")}
          >
            {t("Set Upper Prediction")}
          </Button>
          <Button
            disabled={busy || !record.canSetLowerPrediction}
            size="sm"
            variant="outline"
            onClick={() => onPrediction("lower")}
          >
            {t("Set Lower Prediction")}
          </Button>
        </CardContent>
      </Card>
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
          · {half.durationSeconds.toFixed(1)}s
        </CardDescription>
      </CardHeader>
      <CardContent>
        <Breakdown
          characters={half.characters}
          hiddenCharacterCount={half.hiddenCharacterCount}
          hiddenSkillCount={half.hiddenSkillCount}
          skills={half.skills}
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
}: {
  characters: HistoryCharacter[];
  hiddenCharacterCount?: number;
  hiddenSkillCount?: number;
  skills: HistorySkill[];
  embedded?: boolean;
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
      <CharacterRows title="Characters" rows={characters} />
      <div className="min-w-0">
        <Rows
          title="Skills"
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
  const avatarUrl = historyCharacterAvatarUrl(character.charId);
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
  onCompare,
  onLeftChange,
  onRightChange,
  onUseAdjacent,
}: {
  busy: boolean;
  comparison: HistoryComparison | null;
  leftId: string;
  records: HistoryRecord[];
  rightId: string;
  onCompare: () => void;
  onLeftChange: (value: string) => void;
  onRightChange: (value: string) => void;
  onUseAdjacent: () => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("Compare Records")}</CardTitle>
        <CardDescription>
          {t("Comparison is calculated by the Rust history service.")}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid grid-cols-1 gap-2 @min-[720px]/history:grid-cols-[1fr_1fr_auto]">
          <HistorySelect
            value={leftId}
            records={records}
            onChange={onLeftChange}
            label="Left record"
          />
          <HistorySelect
            value={rightId}
            records={records}
            onChange={onRightChange}
            label="Right record"
          />
          <Button
            disabled={busy || !leftId || !rightId || leftId === rightId}
            onClick={onCompare}
          >
            {t("Compare")}
          </Button>
        </div>
        <Button size="sm" variant="ghost" onClick={onUseAdjacent}>
          {t("Compare selected with adjacent")}
        </Button>
        {comparison && <ComparisonResult comparison={comparison} />}
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
  return (
    <div className="space-y-3 border-t pt-3">
      {(comparison.differentTimeBasis ||
        comparison.differentReactionAccounting) && (
        <Alert>
          <AlertTitle>{t("Comparison settings differ")}</AlertTitle>
          <AlertDescription>
            {t(
              "Time basis or reaction accounting differs between these records.",
            )}
          </AlertDescription>
        </Alert>
      )}
      <div className="grid grid-cols-1 gap-2 @min-[520px]/history:grid-cols-3">
        <Delta label="DPS" value={comparison.totalDpsDelta} />
        <Delta label="Damage" value={comparison.totalDamageDelta} />
        <Delta label="Duration" value={comparison.durationDelta} suffix="s" />
      </div>
      <Rows
        title="Character deltas"
        rows={comparison.characterDeltas.map((row) => ({
          key: String(row.charId),
          title: row.name,
          value: signed(row.deltaDps),
          detail: `${number(row.leftDps)} → ${number(row.rightDps)} DPS`,
        }))}
      />
      <Rows
        title="Skill deltas"
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

function Delta({
  label,
  value,
  suffix = "",
}: {
  label: string;
  value: number;
  suffix?: string;
}) {
  const positive = value >= 0;
  return (
    <div className="rounded-lg bg-muted/60 p-3">
      <p className="text-xs text-muted-foreground">{t(label)}</p>
      <p
        className={cn(
          "mt-1 flex items-center gap-1 font-mono font-semibold",
          positive ? "text-emerald-600" : "text-destructive",
        )}
      >
        {positive ? (
          <ArrowUpRight className="size-4" />
        ) : (
          <ArrowDownRight className="size-4" />
        )}
        {signed(value)}
        {suffix}
      </p>
    </div>
  );
}

function HistoryLoading() {
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 p-5">
      <Skeleton className="h-12 w-full" />
      <div className="grid flex-1 grid-cols-1 gap-4 min-[1280px]:grid-cols-[17rem_1fr]">
        <Skeleton className="h-24 min-[1280px]:h-full" />
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
  return `${value >= 0 ? "+" : ""}${number(value)}`;
}
