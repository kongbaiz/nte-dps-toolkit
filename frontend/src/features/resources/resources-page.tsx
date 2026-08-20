import { useEffect, useMemo, useState } from "react";
import {
  Check,
  CircleCheck,
  Clipboard,
  FileWarning,
  FolderSearch,
  RefreshCw,
  TriangleAlert,
} from "lucide-react";

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
  ResourceCategory,
  ResourceItemSnapshot,
  ResourcesCommandError,
  ResourcesSnapshot,
} from "@/lib/tauri/resources-contract";
import { cn } from "@/lib/utils";

import {
  filterResourceItems,
  resourcesContentKind,
  type ResourceCategoryFilter,
  type ResourceSeverityFilter,
} from "./resources-model";
import { useResources } from "./use-resources";

const CATEGORY_LABELS: Record<ResourceCategory, string> = {
  character: "Character",
  skill: "Skill",
  gameplayEffect: "GE",
  reaction: "Reaction",
  file: "File",
};

export function ResourcesPage() {
  const { state, refreshing, refresh } = useResources();
  const [severity, setSeverity] = useState<ResourceSeverityFilter>("all");
  const [category, setCategory] = useState<ResourceCategoryFilter>("all");
  const [copyState, setCopyState] = useState<"idle" | "copied" | "error">(
    "idle",
  );
  const snapshot = state.status === "ready" ? state.snapshot : null;
  const filteredItems = useMemo(
    () => filterResourceItems(snapshot?.items ?? [], severity, category),
    [category, severity, snapshot],
  );
  const contentKind = resourcesContentKind(
    state.status,
    snapshot?.items.length ?? 0,
    filteredItems.length,
  );

  useEffect(() => {
    if (copyState === "idle") return;
    const timeout = window.setTimeout(() => setCopyState("idle"), 1_600);
    return () => window.clearTimeout(timeout);
  }, [copyState]);

  async function copyReport() {
    if (!snapshot) return;
    try {
      await navigator.clipboard.writeText(snapshot.redactedReport);
      setCopyState("copied");
    } catch {
      setCopyState("error");
    }
  }

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b px-3 py-3 min-[640px]:px-5">
        <div className="min-w-0">
          <h1 className="text-base font-semibold">{t("Resources")}</h1>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t(
              "Audit packaged runtime resources without accessing game exports or resource keys",
            )}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={!snapshot}
            className="min-w-32"
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
          <Button size="sm" disabled={refreshing} onClick={refresh}>
            <RefreshCw
              className={cn(refreshing && "animate-spin")}
              aria-hidden="true"
            />
            {t(refreshing ? "Checking runtime resources" : "Refresh Check")}
          </Button>
        </div>
      </header>

      {snapshot ? <ResourceSummary snapshot={snapshot} /> : null}

      <div className="flex flex-wrap items-center gap-3 border-b bg-muted/15 px-3 py-2.5 min-[640px]:px-5">
        <label className="flex items-center gap-2 text-sm">
          <span className="text-muted-foreground">{t("Level")}</span>
          <select
            className="h-8 rounded-md border bg-background px-2 text-sm outline-none focus:border-ring focus:ring-3 focus:ring-ring/25"
            value={severity}
            onChange={(event) =>
              setSeverity(event.currentTarget.value as ResourceSeverityFilter)
            }
          >
            <option value="all">{t("All Levels")}</option>
            <option value="error">{t("Errors Only")}</option>
            <option value="warning">{t("Warnings Only")}</option>
          </select>
        </label>
        <label className="flex items-center gap-2 text-sm">
          <span className="text-muted-foreground">{t("Category")}</span>
          <select
            className="h-8 rounded-md border bg-background px-2 text-sm outline-none focus:border-ring focus:ring-3 focus:ring-ring/25"
            value={category}
            onChange={(event) =>
              setCategory(event.currentTarget.value as ResourceCategoryFilter)
            }
          >
            <option value="all">{t("All Categories")}</option>
            {Object.entries(CATEGORY_LABELS).map(([value, label]) => (
              <option value={value} key={value}>
                {t(label)}
              </option>
            ))}
          </select>
        </label>
        {snapshot ? (
          <span className="ml-auto text-xs text-muted-foreground tabular-nums">
            {tf("Showing {0} of {1} resource gaps", [
              filteredItems.length.toLocaleString(),
              snapshot.itemCount.toLocaleString(),
            ])}
          </span>
        ) : null}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
        {contentKind === "loading" ? <ResourcesLoading /> : null}
        {contentKind === "error" && state.status === "error" ? (
          <ResourcesLoadError error={state.error} onRetry={refresh} />
        ) : null}
        {contentKind === "clear" ? <ResourcesClear /> : null}
        {contentKind === "filtered-empty" ? <ResourcesFilteredEmpty /> : null}
        {contentKind === "list" ? (
          <div className="mx-auto w-full max-w-[1800px] divide-y">
            {filteredItems.map((item) => (
              <ResourceRow
                key={`${item.severity}:${item.category}:${item.resourceId}:${item.messageKey}:${item.suggestedSource}`}
                item={item}
              />
            ))}
          </div>
        ) : null}
      </div>
    </section>
  );
}

function ResourceSummary({ snapshot }: { snapshot: ResourcesSnapshot }) {
  const summary = [
    {
      label: "Errors",
      value: snapshot.errorCount.toLocaleString(),
      tone: "text-destructive",
    },
    {
      label: "Warnings",
      value: snapshot.warningCount.toLocaleString(),
      tone: "text-amber-700 dark:text-amber-300",
    },
    {
      label: "Characters/Skills",
      value: `${snapshot.counts.characters.toLocaleString()} / ${snapshot.counts.skillDamage.toLocaleString()}`,
      tone: "text-foreground",
    },
    {
      label: "Reaction",
      value: snapshot.counts.reactions.toLocaleString(),
      tone: "text-foreground",
    },
  ];
  return (
    <dl className="grid grid-cols-2 bg-card/35 min-[880px]:grid-cols-4 min-[880px]:divide-x">
      {summary.map((item) => (
        <div
          className="flex min-w-0 items-baseline justify-between gap-3 border-b px-3 py-2.5 min-[640px]:px-5"
          key={item.label}
        >
          <dt className="truncate text-xs text-muted-foreground">
            {t(item.label)}
          </dt>
          <dd
            className={cn(
              "shrink-0 font-mono text-sm font-semibold tabular-nums",
              item.tone,
            )}
          >
            {item.value}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function ResourceRow({ item }: { item: ResourceItemSnapshot }) {
  const error = item.severity === "error";
  return (
    <article
      className={cn(
        "relative grid min-h-16 grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 px-4 py-2.5 hover:bg-muted/25 min-[760px]:grid-cols-[auto_minmax(180px,0.85fr)_minmax(220px,1.35fr)_minmax(180px,0.8fr)] min-[760px]:items-center min-[760px]:gap-4 min-[760px]:px-5",
        "before:absolute before:inset-y-2 before:left-0 before:w-0.5 before:rounded-full",
        error ? "before:bg-destructive" : "before:bg-amber-500",
      )}
      style={{ contentVisibility: "auto", containIntrinsicSize: "64px" }}
    >
      <div
        className={cn(
          "row-span-2 flex size-8 items-center justify-center rounded-md bg-muted",
          error ? "text-destructive" : "text-amber-700 dark:text-amber-300",
        )}
      >
        {error ? (
          <TriangleAlert className="size-4" aria-hidden="true" />
        ) : (
          <FileWarning className="size-4" aria-hidden="true" />
        )}
      </div>
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <Badge variant="outline" className="shrink-0">
            {t(CATEGORY_LABELS[item.category])}
          </Badge>
          <span
            className="truncate text-sm font-medium"
            title={item.displayName}
          >
            {t(item.displayName)}
          </span>
        </div>
        <div
          className="mt-0.5 truncate font-mono text-[11px] text-muted-foreground select-text"
          title={item.resourceId}
        >
          {item.resourceId}
        </div>
      </div>
      <p className="col-start-2 text-xs text-muted-foreground min-[760px]:col-start-auto min-[760px]:text-sm">
        {tf(item.messageKey, item.messageArguments)}
      </p>
      <div className="col-start-2 min-w-0 min-[760px]:col-start-auto">
        <div className="text-[10px] text-muted-foreground">
          {t("Suggested source")}
        </div>
        <code
          className="block truncate text-[11px] text-foreground/75 select-text"
          title={item.suggestedSource}
        >
          {item.suggestedSource}
        </code>
      </div>
    </article>
  );
}

function ResourcesLoading() {
  return (
    <div className="mx-auto flex w-full max-w-[1800px] flex-col gap-1 px-3 py-4 min-[640px]:px-5">
      {Array.from({ length: 9 }, (_, index) => (
        <Skeleton key={index} className="h-16 w-full rounded-md" />
      ))}
    </div>
  );
}

function ResourcesLoadError({
  error,
  onRetry,
}: {
  error: ResourcesCommandError;
  onRetry: () => void;
}) {
  return (
    <div className="mx-auto w-full max-w-3xl p-5">
      <Alert variant="destructive">
        <TriangleAlert className="size-4" aria-hidden="true" />
        <AlertTitle>{t("Runtime resources could not be checked")}</AlertTitle>
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

function ResourcesClear() {
  return (
    <Empty className="min-h-full border-0">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <CircleCheck aria-hidden="true" />
        </EmptyMedia>
        <EmptyTitle>{t("Runtime resource coverage is complete")}</EmptyTitle>
        <EmptyDescription>
          {t("No packaged resource gaps were found.")}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}

function ResourcesFilteredEmpty() {
  return (
    <Empty className="min-h-full border-0">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <FolderSearch aria-hidden="true" />
        </EmptyMedia>
        <EmptyTitle>
          {t("No resource gaps under the current filter")}
        </EmptyTitle>
        <EmptyDescription>
          {t("Choose another level or category to inspect remaining gaps.")}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}
