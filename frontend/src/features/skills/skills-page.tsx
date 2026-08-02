import {
  AlertTriangle,
  BarChart3,
  Check,
  Copy,
  GitBranch,
  ListChecks,
  RefreshCw,
  Sparkles,
  UsersRound,
  X,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
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
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { characterAvatarUrl } from "@/lib/character-avatar";
import { t, tf } from "@/lib/i18n";
import type {
  SkillsCharacter,
  SkillsCommandError,
  SkillsDiagnostics,
  SkillsRow,
  SkillsScope,
  SkillsSnapshot,
} from "@/lib/tauri/skills-contract";
import { cn } from "@/lib/utils";

import {
  copyGameplayEffectIndex,
  skillTechnicalDetails,
} from "./skills-interactions";
import {
  buildSkillCharacterSummaries,
  skillDamageShare,
  skillRowsForCharacter,
  skillViewMetrics,
  type SkillCharacterSummary,
} from "./skills-ui-model";
import { useSkills } from "./use-skills";

const SCOPES: Array<{ id: SkillsScope; labelKey: string }> = [
  { id: "all", labelKey: "Whole Battle" },
  { id: "upper", labelKey: "First Half" },
  { id: "lower", labelKey: "Second Half" },
];
const NUMBER_FORMATTER = new Intl.NumberFormat();

export function SkillsPage() {
  const { state, scope, setScope, streamError, clearStreamError, retry } =
    useSkills();
  const snapshot = state.status === "ready" ? state.snapshot : null;
  const [selectedCharacterId, setSelectedCharacterId] = useState<number | null>(
    null,
  );

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

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background">
      {streamError ? (
        <SkillsStreamError error={streamError} onDismiss={clearStreamError} />
      ) : null}
      <header className="flex flex-wrap items-center justify-between gap-3 border-b bg-background px-3 py-3 min-[640px]:px-5">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-base font-semibold">{t("Skills")}</h1>
            <Badge variant="outline" className="gap-1.5">
              <span className="size-1.5 rounded-full bg-emerald-500" />
              {t("Live")}
            </Badge>
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t("Skill attribution follows the current capture")}
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
              onClick={() => setScope(item.id)}
            >
              {t(item.labelKey)}
            </Button>
          ))}
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-3 min-[900px]:p-4">
        <div className="mx-auto flex min-h-full w-full max-w-[1700px] flex-col gap-3.5">
          {state.status === "loading" ? <SkillsLoading /> : null}
          {state.status === "error" ? (
            <SkillsLoadError error={state.error} onRetry={retry} />
          ) : null}
          {snapshot && !snapshot.hasData ? <SkillsEmpty /> : null}
          {snapshot?.hasData ? (
            <SkillsReady
              snapshot={snapshot}
              selectedCharacterId={selectedCharacterId}
              onSelectCharacter={setSelectedCharacterId}
            />
          ) : null}
        </div>
      </div>
    </section>
  );
}

function SkillsReady({
  snapshot,
  selectedCharacterId,
  onSelectCharacter,
}: {
  snapshot: SkillsSnapshot;
  selectedCharacterId: number | null;
  onSelectCharacter: (characterId: number | null) => void;
}) {
  const characters = useMemo(
    () => buildSkillCharacterSummaries(snapshot),
    [snapshot],
  );
  const characterById = useMemo(
    () =>
      new Map(
        snapshot.characters.map((character) => [character.id, character]),
      ),
    [snapshot.characters],
  );
  const visibleRows = useMemo(
    () => skillRowsForCharacter(snapshot, selectedCharacterId),
    [selectedCharacterId, snapshot],
  );
  const visibleMetrics = skillViewMetrics(visibleRows);
  const selectedCharacter = characters.find(
    (character) => character.id === selectedCharacterId,
  );
  const diagnostics = snapshot.diagnostics;

  return (
    <>
      <div className="grid grid-cols-2 border-y border-border/70 bg-background/25 [&>*:nth-child(odd)]:border-r [&>*:nth-child(-n+2)]:border-b min-[880px]:grid-cols-4 min-[880px]:[&>*]:border-b-0 min-[880px]:[&>*:not(:last-child)]:border-r">
        <Metric
          icon={BarChart3}
          label="Attributed Damage"
          value={number(visibleMetrics.damage)}
          prominent
        />
        <Metric
          icon={ListChecks}
          label="Skill Entries"
          value={String(visibleMetrics.entries)}
          prominent
        />
        <Metric
          icon={AlertTriangle}
          label="Pending Mapping"
          value={diagnostics.unmappedSkillHits}
          warning={diagnostics.unmappedSkillHits !== "0"}
        />
        <Metric
          icon={GitBranch}
          label="Candidate Direction"
          value={diagnostics.unknownDirectionHits}
          warning={diagnostics.unknownDirectionHits !== "0"}
        />
      </div>

      <div className="grid min-h-[30rem] flex-1 overflow-hidden border-y border-border/70 bg-background/25 min-[920px]:grid-cols-[17rem_minmax(0,1fr)]">
        <aside className="border-b border-border/70 p-2.5 min-[920px]:border-r min-[920px]:border-b-0">
          <div className="mb-2 flex items-center justify-between gap-3 px-1.5">
            <h2 className="text-sm font-semibold">{t("Character")}</h2>
            <span className="font-mono text-[11px] text-muted-foreground">
              {characters.length}
            </span>
          </div>
          <div className="flex gap-1.5 overflow-x-auto pb-1 min-[920px]:max-h-[calc(100vh-15rem)] min-[920px]:flex-col min-[920px]:overflow-x-hidden min-[920px]:overflow-y-auto">
            <button
              type="button"
              aria-pressed={selectedCharacterId === null}
              onClick={() => onSelectCharacter(null)}
              className={cn(
                "flex min-w-44 items-center gap-2.5 rounded-md px-2.5 py-2 text-left transition-colors min-[920px]:min-w-0",
                selectedCharacterId === null
                  ? "bg-primary text-primary-foreground hover:bg-primary"
                  : "hover:bg-muted/70",
              )}
            >
              <span className="flex size-9 shrink-0 items-center justify-center rounded-md bg-background/70 text-muted-foreground">
                <UsersRound className="size-4" aria-hidden="true" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium">
                  {t("Whole Team")}
                </span>
                <span
                  className={cn(
                    "block truncate font-mono text-[11px]",
                    selectedCharacterId === null
                      ? "text-primary-foreground/70"
                      : "text-muted-foreground",
                  )}
                >
                  {number(snapshot.totalDamage)} · {snapshot.totalHits}{" "}
                  {t("hits")}
                </span>
              </span>
            </button>
            {characters.map((character) => (
              <CharacterButton
                character={character}
                key={character.id}
                selected={selectedCharacterId === character.id}
                onSelect={() => onSelectCharacter(character.id)}
              />
            ))}
          </div>
        </aside>

        <div className="flex min-w-0 flex-col p-3 min-[720px]:p-4">
          <div className="mb-3 flex flex-wrap items-end justify-between gap-2 border-b border-border/60 pb-3">
            <div className="min-w-0">
              <p className="text-xs text-muted-foreground">{t("Skill")}</p>
              <h2 className="truncate text-base font-semibold">
                {selectedCharacter?.name ?? t("Whole Team")}
              </h2>
            </div>
            <div className="flex items-center gap-3 font-mono text-xs text-muted-foreground">
              <span>{number(visibleMetrics.damage)}</span>
              <span>
                {visibleMetrics.entries} {t("Skill Entries")}
              </span>
            </div>
          </div>

          <div className="flex flex-1 flex-col gap-1.5">
            {visibleRows.length === 0 ? (
              <p className="py-12 text-center text-sm text-muted-foreground">
                {t("No skill attribution for this character yet")}
              </p>
            ) : (
              visibleRows.map((row) => (
                <SkillRow
                  character={characterFor(row, characterById)}
                  key={row.id}
                  row={row}
                  totalDamage={visibleMetrics.damage}
                />
              ))
            )}
          </div>

          <details className="mt-3 border-t border-border/70 pt-2">
            <summary className="flex cursor-pointer list-none items-center gap-2 rounded-md px-2 py-2 text-sm font-medium hover:bg-muted/55">
              <AlertTriangle
                className="size-4 text-amber-600 dark:text-amber-400"
                aria-hidden="true"
              />
              {t("Pending Mapping Diagnostics")}
              <span className="ml-auto font-mono text-xs text-muted-foreground">
                {diagnostics.unmappedSkillRows}
              </span>
            </summary>
            <div className="grid gap-x-6 gap-y-2 px-2 pt-2 pb-1 text-xs min-[680px]:grid-cols-3">
              <Diagnostic
                label="Unknown Characters"
                value={`${diagnostics.unknownCharacterCount} / ${diagnostics.unknownCharacterHits}`}
              />
              <Diagnostic
                label="Candidate Direction"
                value={`${diagnostics.unknownDirectionHits} / ${number(diagnostics.unknownDirectionDamage)}`}
              />
              <Diagnostic
                label="Pending Skills"
                value={tf("{} kinds / {} hits", [
                  diagnostics.unmappedSkillRows,
                  diagnostics.unmappedSkillHits,
                ])}
              />
              {diagnostics.unmappedGameplayEffects.length > 0 ? (
                <div className="min-[680px]:col-span-3">
                  <p className="mb-1 text-muted-foreground">
                    {t("Unmapped GE")}
                  </p>
                  <div className="flex flex-wrap gap-1.5">
                    {diagnostics.unmappedGameplayEffects.map((effect) => (
                      <UnmappedGameplayEffect
                        effect={effect}
                        key={effect.index}
                      />
                    ))}
                  </div>
                </div>
              ) : null}
            </div>
          </details>
        </div>
      </div>
    </>
  );
}

function SkillsLoading() {
  return (
    <div aria-label={t("Loading Skills")} className="flex flex-col gap-3">
      <div className="grid grid-cols-2 gap-px border-y bg-border min-[880px]:grid-cols-4">
        {Array.from({ length: 4 }, (_, index) => (
          <div
            className="flex items-center gap-3 bg-background px-3 py-3"
            key={index}
          >
            <Skeleton className="size-9 rounded-md" />
            <div className="flex flex-1 flex-col gap-2">
              <Skeleton className="h-3 w-20" />
              <Skeleton className="h-4 w-28" />
            </div>
          </div>
        ))}
      </div>
      <Skeleton className="h-[30rem] w-full rounded-none" />
    </div>
  );
}

function SkillsEmpty() {
  return (
    <Empty className="min-h-[24rem] border">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Sparkles aria-hidden="true" />
        </EmptyMedia>
        <EmptyTitle>{t("No skill attribution data yet")}</EmptyTitle>
        <EmptyDescription>
          {t("Start capture or import a replay to attribute damage to skills.")}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}

function SkillsLoadError({
  error,
  onRetry,
}: {
  error: SkillsCommandError;
  onRetry: () => void;
}) {
  return (
    <Alert variant="destructive">
      <AlertTriangle aria-hidden="true" />
      <AlertTitle>{t("Skills data could not be loaded")}</AlertTitle>
      <AlertDescription>
        {tf(error.messageKey, error.messageArguments)}
      </AlertDescription>
      <AlertAction>
        <Button size="sm" variant="outline" onClick={onRetry}>
          <RefreshCw className="size-3.5" aria-hidden="true" />
          {t("Retry")}
        </Button>
      </AlertAction>
    </Alert>
  );
}

function SkillsStreamError({
  error,
  onDismiss,
}: {
  error: SkillsCommandError;
  onDismiss: () => void;
}) {
  return createPortal(
    <div className="pointer-events-none fixed inset-x-0 top-4 z-[100] flex justify-center px-4">
      <Alert
        className="pointer-events-auto w-full max-w-lg bg-background pr-10 shadow-xl"
        variant="destructive"
      >
        <AlertTriangle aria-hidden="true" />
        <AlertTitle>{t("Skills stream interrupted")}</AlertTitle>
        <AlertDescription>
          {tf(error.messageKey, error.messageArguments)}
        </AlertDescription>
        <AlertAction>
          <Button
            aria-label={t("Dismiss")}
            size="icon-xs"
            variant="ghost"
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

function CharacterButton({
  character,
  selected,
  onSelect,
}: {
  character: SkillCharacterSummary;
  selected: boolean;
  onSelect: () => void;
}) {
  const avatar = characterAvatarUrl(character.id);
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      className={cn(
        "flex min-w-52 items-center gap-2.5 rounded-md px-2.5 py-2 text-left transition-colors min-[920px]:min-w-0",
        selected
          ? "bg-primary text-primary-foreground hover:bg-primary"
          : "hover:bg-muted/70",
      )}
    >
      <span
        className="h-9 w-1 shrink-0 rounded-full"
        style={{ backgroundColor: character.color }}
      />
      {avatar ? (
        <img
          alt=""
          className="size-9 shrink-0 rounded-md object-cover"
          draggable={false}
          src={avatar}
        />
      ) : (
        <span className="flex size-9 shrink-0 items-center justify-center rounded-md bg-muted font-semibold">
          {character.name.slice(0, 1)}
        </span>
      )}
      <span className="min-w-0 flex-1">
        <span className="flex items-center justify-between gap-2">
          <span className="truncate text-sm font-medium">{character.name}</span>
          <span className="font-mono text-[11px]">
            {(character.share * 100).toFixed(1)}%
          </span>
        </span>
        <span
          className={cn(
            "block truncate font-mono text-[11px]",
            selected ? "text-primary-foreground/70" : "text-muted-foreground",
          )}
        >
          {number(character.damage)} · {character.entries} {t("Skill Entries")}
        </span>
      </span>
    </button>
  );
}

function SkillRow({
  row,
  character,
  totalDamage,
}: {
  row: SkillsRow;
  character: SkillsCharacter;
  totalDamage: number;
}) {
  const share = skillDamageShare(row.damage, totalDamage);
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <article
            aria-label={row.name}
            className="group relative isolate overflow-hidden rounded-md border border-transparent px-3 py-2.5 transition-colors hover:border-border hover:bg-muted/30 focus-visible:border-ring focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
            tabIndex={0}
          />
        }
      >
        <span
          className="absolute inset-y-0 left-0 -z-10 opacity-[0.09] transition-opacity group-hover:opacity-[0.14]"
          style={{
            backgroundColor: character.color,
            width: `${Math.max(0.75, share * 100)}%`,
          }}
        />
        <span
          className="absolute inset-y-0 left-0 w-0.5"
          style={{ backgroundColor: character.color }}
        />
        <div className="grid min-w-0 items-center gap-x-4 gap-y-1 min-[680px]:grid-cols-[minmax(0,1fr)_auto_auto]">
          <div className="min-w-0">
            <p className="truncate text-sm font-medium">{row.name}</p>
            <p className="truncate text-xs text-muted-foreground">
              {character.name} · {row.category}
              {row.followUp ? ` · ${t("follow-up")}` : ""}
            </p>
          </div>
          <div className="font-mono text-sm font-semibold min-[680px]:text-right">
            {number(row.damage)}
          </div>
          <div className="flex items-center gap-2 font-mono text-xs text-muted-foreground min-[680px]:w-28 min-[680px]:justify-end">
            <span>{(share * 100).toFixed(1)}%</span>
            <span>·</span>
            <span>{tf("{} hits", [row.hits])}</span>
          </div>
        </div>
      </TooltipTrigger>
      <TooltipContent
        align="start"
        className="w-72 max-w-[calc(100vw-2rem)] flex-col items-stretch gap-1.5 py-2.5"
        side="top"
      >
        <SkillDetail label="Character" value={character.name} />
        <SkillDetail label="Category" value={row.category} />
        <SkillDetail label="Damage" value={number(row.damage)} mono />
        <SkillDetail label="Hits" value={row.hits} mono />
        {skillTechnicalDetails(row).map((detail) => (
          <SkillDetail
            key={detail.label}
            label={detail.label}
            value={detail.value}
            mono
          />
        ))}
      </TooltipContent>
    </Tooltip>
  );
}

function SkillDetail({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="grid grid-cols-[5rem_minmax(0,1fr)] gap-2">
      <span className="text-background/65">{t(label)}</span>
      <span className={cn("break-all text-right", mono && "font-mono")}>
        {value}
      </span>
    </div>
  );
}

function UnmappedGameplayEffect({
  effect,
}: {
  effect: SkillsDiagnostics["unmappedGameplayEffects"][number];
}) {
  const [copyStatus, setCopyStatus] = useState<"idle" | "copied" | "error">(
    "idle",
  );

  useEffect(() => {
    if (copyStatus === "idle") return;
    const timeout = window.setTimeout(() => setCopyStatus("idle"), 1600);
    return () => window.clearTimeout(timeout);
  }, [copyStatus]);

  const copyIndex = async () => {
    try {
      await copyGameplayEffectIndex(effect.index);
      setCopyStatus("copied");
    } catch {
      setCopyStatus("error");
    }
  };
  const statusLabel =
    copyStatus === "copied"
      ? "Copied"
      : copyStatus === "error"
        ? "Copy failed"
        : "Copy";

  return (
    <span className="inline-flex items-center rounded-md bg-muted/55 pl-2 font-mono">
      <span className="py-1">
        {effect.index} · {tf("{} hits", [effect.hits])} ·{" "}
        {number(effect.damage)}
      </span>
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              aria-label={t(statusLabel)}
              className={cn(
                "ml-1 size-7 rounded-l-none",
                copyStatus === "copied" &&
                  "text-emerald-700 dark:text-emerald-400",
                copyStatus === "error" && "text-destructive",
              )}
              size="icon-xs"
              type="button"
              variant="ghost"
              onClick={() => void copyIndex()}
            />
          }
        >
          {copyStatus === "copied" ? (
            <Check aria-hidden="true" />
          ) : copyStatus === "error" ? (
            <AlertTriangle aria-hidden="true" />
          ) : (
            <Copy aria-hidden="true" />
          )}
        </TooltipTrigger>
        <TooltipContent>{t(statusLabel)}</TooltipContent>
      </Tooltip>
      <span aria-live="polite" className="sr-only">
        {copyStatus === "idle" ? "" : t(statusLabel)}
      </span>
    </span>
  );
}

function Metric({
  icon: Icon,
  label,
  value,
  prominent = false,
  warning = false,
}: {
  icon: LucideIcon;
  label: string;
  value: string;
  prominent?: boolean;
  warning?: boolean;
}) {
  return (
    <div className="flex min-w-0 items-center gap-3 border-border/60 px-3 py-3">
      <span
        className={cn(
          "flex size-9 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground",
          warning && "bg-amber-500/10 text-amber-700 dark:text-amber-400",
        )}
      >
        <Icon className="size-4" aria-hidden="true" />
      </span>
      <span className="min-w-0">
        <span className="block truncate text-xs text-muted-foreground">
          {t(label)}
        </span>
        <span
          className={cn(
            "block truncate font-mono text-sm font-semibold",
            prominent && "text-base",
            warning && "text-amber-700 dark:text-amber-400",
          )}
        >
          {value}
        </span>
      </span>
    </div>
  );
}

function Diagnostic({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3 border-b border-border/50 py-1.5">
      <span className="text-muted-foreground">{t(label)}</span>
      <span className="font-mono">{value}</span>
    </div>
  );
}

function characterFor(
  row: SkillsRow,
  characters: ReadonlyMap<number, SkillsCharacter>,
): SkillsCharacter {
  return (
    characters.get(row.characterId) ?? {
      id: row.characterId,
      name: row.characterName,
      color: "#737373",
      damage: row.damage,
      entries: 1,
    }
  );
}

function number(value: number): string {
  return NUMBER_FORMATTER.format(Math.round(value));
}
