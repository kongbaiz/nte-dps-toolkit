import { useMemo, useState } from "react";
import {
  Activity,
  Backpack,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronsLeft,
  Clipboard,
  Code2,
  Copy,
  Folder,
  FolderOpen,
  History,
  LockKeyhole,
  Minus,
  Puzzle,
  Radio,
  RefreshCw,
  Settings,
  Sparkles,
  Timeline,
  TriangleAlert,
  UserRound,
  type LucideIcon,
} from "lucide-react";

import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert";
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
import { cn } from "@/lib/utils";
import { t, tf } from "@/lib/i18n";
import type {
  ModStudioCommandError,
  ModStudioDocumentSummary,
} from "@/lib/tauri/mod-studio-contract";

import { highlightModSource } from "./mod-source-highlight";
import { useModStudio } from "./use-mod-studio";

const MAX_MOD_SOURCE_BYTES = 16_384;

interface ConsoleNavItem {
  labelKey: string;
  icon: LucideIcon;
  active?: boolean;
}

const CONSOLE_NAV_GROUPS: Array<{
  labelKey: string;
  items: ConsoleNavItem[];
}> = [
  {
    labelKey: "Common",
    items: [
      { labelKey: "Settings", icon: Settings },
      { labelKey: "History", icon: History },
    ],
  },
  {
    labelKey: "Review",
    items: [
      { labelKey: "Timeline", icon: Timeline },
      { labelKey: "Skills", icon: Sparkles },
      { labelKey: "Console Loadout", icon: Backpack },
      { labelKey: "Mod Studio", icon: Puzzle, active: true },
    ],
  },
  {
    labelKey: "Advanced",
    items: [
      { labelKey: "Character Data", icon: UserRound },
      { labelKey: "Encrypted INI", icon: LockKeyhole },
      { labelKey: "Packets", icon: Radio },
      { labelKey: "Resources", icon: Folder },
      { labelKey: "Diagnostics", icon: Activity },
    ],
  },
];

export function ModStudioPage() {
  const { state, refresh, chooseDocument } = useModStudio();
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [consoleCollapsed, setConsoleCollapsed] = useState(false);
  const selectedSummary =
    state.status === "ready"
      ? state.workspace.documents.find(
          (document) => document.id === state.selectedId,
        )
      : undefined;

  return (
    <main className="console-light flex h-screen min-h-0 w-screen overflow-hidden bg-background text-foreground select-none">
      <ConsoleSidebar
        collapsed={sidebarCollapsed}
        onToggle={() => setSidebarCollapsed((collapsed) => !collapsed)}
      />
      <section className="flex min-w-0 flex-1 flex-col p-3">
        <ModStudioHeader onRefresh={refresh} />
        <div className="mt-3 grid min-h-0 flex-1 grid-cols-[clamp(12rem,21vw,17rem)_minmax(0,1fr)] overflow-hidden border bg-card max-[900px]:grid-cols-[12rem_minmax(0,1fr)]">
          <ExplorerPane
            state={state}
            selectedId={state.status === "ready" ? state.selectedId : null}
            onSelect={chooseDocument}
            onRefresh={refresh}
          />
          <section className="flex min-h-0 min-w-0 flex-col">
            {state.status === "ready" ? (
              <EditorPane
                selectedId={state.selectedId}
                selectedSummary={selectedSummary}
                document={state.document}
                onRetry={refresh}
                consoleCollapsed={consoleCollapsed}
                onToggleConsole={() =>
                  setConsoleCollapsed((collapsed) => !collapsed)
                }
              />
            ) : (
              <EditorPlaceholder loading={state.status === "loading"} />
            )}
          </section>
        </div>
      </section>
    </main>
  );
}

interface ConsoleSidebarProps {
  collapsed: boolean;
  onToggle: () => void;
}

function ConsoleSidebar({ collapsed, onToggle }: ConsoleSidebarProps) {
  return (
    <aside
      className={cn(
        "flex w-52 shrink-0 flex-col border-r bg-sidebar px-3 py-3 text-sidebar-foreground transition-[width] duration-150 max-[900px]:w-14 max-[900px]:px-1.5",
        collapsed && "w-14 px-1.5",
      )}
      aria-label={t("Console navigation")}
    >
      <Tooltip>
        <TooltipTrigger
          render={
            <button
              type="button"
              className="mb-2 flex h-9 w-full items-center justify-center gap-2 rounded-md border bg-card text-sm text-muted-foreground hover:bg-muted"
              aria-label={t(collapsed ? "Expand sidebar" : "Collapse sidebar")}
              onClick={onToggle}
            />
          }
        >
          {collapsed ? (
            <ChevronLeft className="size-4 rotate-180" aria-hidden="true" />
          ) : (
            <>
              <ChevronsLeft className="size-4" aria-hidden="true" />
              <span className="max-[900px]:sr-only">{t("Collapse")}</span>
            </>
          )}
        </TooltipTrigger>
        <TooltipContent>
          {t(collapsed ? "Expand sidebar" : "Collapse sidebar")}
        </TooltipContent>
      </Tooltip>

      <nav className="min-h-0 overflow-y-auto">
        {CONSOLE_NAV_GROUPS.map((group) => (
          <div className="mb-3" key={group.labelKey}>
            <p
              className={cn(
                "mb-1 px-2 text-[11px] text-muted-foreground max-[900px]:sr-only",
                collapsed && "sr-only",
              )}
            >
              {t(group.labelKey)}
            </p>
            <div className="flex flex-col gap-0.5">
              {group.items.map((item) => (
                <ConsoleNavRow
                  key={item.labelKey}
                  item={item}
                  labelHidden={collapsed}
                />
              ))}
            </div>
          </div>
        ))}
      </nav>
    </aside>
  );
}

function ConsoleNavRow({
  item,
  labelHidden,
}: {
  item: ConsoleNavItem;
  labelHidden: boolean;
}) {
  const Icon = item.icon;
  const row = (
    <div
      className={cn(
        "flex h-9 items-center gap-3 rounded-md px-2 text-sm text-muted-foreground max-[900px]:justify-center max-[900px]:px-0",
        labelHidden && "justify-center px-0",
        item.active &&
          "bg-sidebar-primary text-sidebar-primary-foreground shadow-sm",
      )}
      aria-current={item.active ? "page" : undefined}
      aria-disabled={!item.active}
    >
      <Icon className="size-[18px] shrink-0" aria-hidden="true" />
      <span
        className={cn("truncate max-[900px]:sr-only", labelHidden && "sr-only")}
      >
        {t(item.labelKey)}
      </span>
    </div>
  );

  if (!labelHidden) {
    return row;
  }
  return (
    <Tooltip>
      <TooltipTrigger render={row} />
      <TooltipContent side="right">{t(item.labelKey)}</TooltipContent>
    </Tooltip>
  );
}

function ModStudioHeader({
  onRefresh,
}: {
  onRefresh: () => void | Promise<void>;
}) {
  return (
    <header className="border bg-card px-3 py-2.5">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h1 className="font-heading text-xl font-medium">
          {t("Mod Code Editor")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {t("Write, validate and manage NTE C++ Mods in one place.")}
        </p>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2 text-sm">
        <span className="text-muted-foreground">{t("Game client")}</span>
        <select
          className="h-8 rounded-md border bg-background px-3 text-sm"
          aria-label={t("Game client")}
          defaultValue="cn"
          disabled
        >
          <option value="cn">{t("China client")}</option>
        </select>
        <Button
          variant="outline"
          size="sm"
          className="h-8"
          onClick={() => void onRefresh()}
        >
          <RefreshCw className="size-4" aria-hidden="true" />
          {t("Refresh")}
        </Button>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                variant="outline"
                size="sm"
                className="h-8"
                disabled
                aria-label={t("Open Mod folder")}
              />
            }
          >
            <FolderOpen aria-hidden="true" />
            {t("Open Mod folder")}
          </TooltipTrigger>
          <TooltipContent>
            {t("Folder command will be connected in the next migration slice.")}
          </TooltipContent>
        </Tooltip>
        <span className="mx-1 h-5 w-px bg-border" aria-hidden="true" />
        <TriangleAlert
          className="size-4 text-[var(--console-warning)]"
          aria-hidden="true"
        />
        <span className="font-medium">{t("In-game Mod loader")}</span>
        <span className="text-muted-foreground">
          {t(
            "Loader status is not connected in this read-only migration slice.",
          )}
        </span>
        <label className="ml-auto flex items-center gap-1 text-muted-foreground">
          <input type="checkbox" checked={false} disabled />
          {t("Enable")}
        </label>
      </div>
    </header>
  );
}

type ModStudioState = ReturnType<typeof useModStudio>["state"];

interface ExplorerPaneProps {
  state: ModStudioState;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onRefresh: () => void | Promise<void>;
}

function ExplorerPane({
  state,
  selectedId,
  onSelect,
  onRefresh,
}: ExplorerPaneProps) {
  return (
    <aside className="flex min-h-0 flex-col border-r bg-[var(--console-explorer)]">
      <div className="flex h-10 shrink-0 items-center border-b px-3 text-xs font-semibold uppercase text-muted-foreground">
        {t("Explorer")}
      </div>
      <div className="flex h-10 shrink-0 items-center gap-2 px-3 text-sm font-medium">
        <ChevronDown className="size-4" aria-hidden="true" />
        <FolderOpen
          className="size-4 text-muted-foreground"
          aria-hidden="true"
        />
        {t("NTE Mods").toUpperCase()}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {state.status === "loading" ? <ExplorerLoading /> : null}
        {state.status === "error" ? (
          <div className="p-3">
            <WorkspaceError error={state.error} onRetry={onRefresh} compact />
          </div>
        ) : null}
        {state.status === "empty" ? (
          <Empty className="m-3 min-h-48 border bg-background">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <FolderOpen aria-hidden="true" />
              </EmptyMedia>
              <EmptyTitle>{t("No Mods in this workspace")}</EmptyTitle>
              <EmptyDescription>
                {t(
                  "Add .nte files under plugins/nte-mods, then refresh this page.",
                )}
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : null}
        {state.status === "ready"
          ? state.workspace.documents.map((document) => (
              <DocumentItem
                key={document.id}
                document={document}
                selected={document.id === selectedId}
                onSelect={() => onSelect(document.id)}
              />
            ))
          : null}
      </div>
      <div className="shrink-0 border-t bg-card p-3">
        <label className="block text-[11px] font-medium uppercase text-muted-foreground">
          {t("New Mod ID")}
          <input
            className="mt-1 h-8 w-full rounded-md border bg-background px-2 text-sm"
            placeholder="character-telemetry"
            disabled
          />
        </label>
        <Button variant="outline" size="sm" className="mt-2 h-8" disabled>
          + {t("Create Mod")}
        </Button>
      </div>
    </aside>
  );
}

interface DocumentItemProps {
  document: ModStudioDocumentSummary;
  selected: boolean;
  onSelect: () => void;
}

function DocumentItem({ document, selected, onSelect }: DocumentItemProps) {
  return (
    <button
      type="button"
      aria-current={selected ? "page" : undefined}
      className={cn(
        "relative flex h-10 w-full items-center gap-2 px-3 text-left text-sm outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
        selected && "bg-muted",
      )}
      onClick={onSelect}
    >
      {selected ? (
        <span
          className="absolute inset-y-0 left-0 w-0.5 bg-foreground"
          aria-hidden="true"
        />
      ) : null}
      <span
        className={cn(
          "flex size-4 shrink-0 items-center justify-center rounded-full border text-[10px]",
          document.enabled && "border-foreground",
        )}
        aria-hidden="true"
      >
        {document.enabled ? <Check className="size-3" /> : null}
      </span>
      <Code2
        className="size-4 shrink-0 text-muted-foreground"
        aria-hidden="true"
      />
      <span className="min-w-0 flex-1 truncate font-mono">
        {document.id}.nte
      </span>
      <span className="sr-only">
        {tf("{0} lines · {1} bytes", [
          document.lineCount.toString(),
          document.sourceBytes.toString(),
        ])}
      </span>
    </button>
  );
}

function ExplorerLoading() {
  return (
    <div
      className="flex flex-col gap-2 p-3"
      aria-label={t("Loading Mod workspace")}
    >
      {Array.from({ length: 4 }, (_, index) => (
        <Skeleton key={index} className="h-8 w-full" />
      ))}
    </div>
  );
}

type ReadyState = Extract<ModStudioState, { status: "ready" }>;

interface EditorPaneProps {
  selectedId: string;
  selectedSummary?: ModStudioDocumentSummary;
  document: ReadyState["document"];
  onRetry: () => void | Promise<void>;
  consoleCollapsed: boolean;
  onToggleConsole: () => void;
}

function EditorPane({
  selectedId,
  selectedSummary,
  document,
  onRetry,
  consoleCollapsed,
  onToggleConsole,
}: EditorPaneProps) {
  return (
    <>
      <div className="flex h-11 shrink-0 items-center gap-2 border-b px-3">
        <Code2 className="size-4 text-muted-foreground" aria-hidden="true" />
        <span className="font-mono text-sm">{selectedId}.nte</span>
        <div className="ml-auto flex gap-2">
          <Button variant="outline" size="sm" className="h-8" disabled>
            {t("Save Mod")}
          </Button>
          <Button variant="outline" size="sm" className="h-8" disabled>
            {t("Revert changes")}
          </Button>
        </div>
      </div>
      <div className="flex h-10 shrink-0 items-center gap-2 border-b px-2 text-sm font-medium">
        <ChevronDown className="size-4 -rotate-90" aria-hidden="true" />
        {t("Getting started")}
      </div>
      <div className="flex min-h-11 shrink-0 flex-wrap items-center gap-x-2 gap-y-1 border-b px-3 text-xs text-muted-foreground">
        <span>{t("NTE Mods")}</span>
        <ChevronLeft className="size-3 rotate-180" aria-hidden="true" />
        <span className="font-mono text-foreground">{selectedId}.nte</span>
        {document.status === "ready"
          ? extractCapabilityLabels(document.document.source).map((label) => (
              <span
                key={label}
                className="rounded-sm bg-muted px-1.5 py-0.5 text-foreground"
              >
                {label}
              </span>
            ))
          : null}
      </div>
      <div className="flex min-h-0 flex-1 flex-col">
        {document.status === "loading" ? (
          <div className="min-h-0 flex-1 overflow-hidden p-4">
            <SourceLoading />
          </div>
        ) : null}
        {document.status === "error" ? (
          <div className="min-h-0 flex-1 overflow-auto p-4">
            <DocumentError error={document.error} onRetry={onRetry} />
          </div>
        ) : null}
        {document.status === "ready" ? (
          <SourceEditor
            source={document.document.source}
            summary={selectedSummary}
          />
        ) : null}
      </div>
      <RuntimeConsole collapsed={consoleCollapsed} onToggle={onToggleConsole} />
    </>
  );
}

function SourceEditor({
  source,
  summary,
}: {
  source: string;
  summary?: ModStudioDocumentSummary;
}) {
  const lines = useMemo(() => highlightModSource(source), [source]);
  const sourceBytes =
    summary?.sourceBytes ?? new TextEncoder().encode(source).length;

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-[var(--console-editor)]">
      <div
        className="min-h-0 flex-1 overflow-auto py-2 font-mono text-[13px] leading-6 select-text"
        data-mod-source
        tabIndex={0}
        aria-label={t("Read-only source preview")}
      >
        <code className="block min-w-max">
          {lines.map((line) => (
            <span className="flex min-h-6" key={line.number}>
              <span
                className="sticky left-0 w-12 shrink-0 bg-[var(--console-editor)] pr-3 text-right text-[var(--editor-line-number)] select-none"
                aria-hidden="true"
              >
                {line.number}
              </span>
              <span className="pr-5 whitespace-pre">
                {line.tokens.map((token, index) => (
                  <span
                    className={`mod-token-${token.kind}`}
                    key={`${line.number}-${index}`}
                  >
                    {token.text}
                  </span>
                ))}
              </span>
            </span>
          ))}
        </code>
      </div>
      <div className="flex h-7 shrink-0 items-center bg-[var(--console-status)] px-2 text-[11px] text-white">
        <Check className="mr-1 size-3.5" aria-hidden="true" />
        <span>{t("Source is valid")}</span>
        <div className="ml-auto flex items-center divide-x divide-white/30">
          <span className="px-3">{tf("Ln {0}, Col {1}", ["1", "1"])}</span>
          <span className="px-3">
            {tf("{0} / {1} bytes", [
              sourceBytes.toString(),
              MAX_MOD_SOURCE_BYTES.toString(),
            ])}
          </span>
          <span className="px-3">{t("Ctrl+Space")}</span>
          <span className="pl-3">{t("NTE C++")}</span>
        </div>
      </div>
    </div>
  );
}

function RuntimeConsole({
  collapsed,
  onToggle,
}: {
  collapsed: boolean;
  onToggle: () => void;
}) {
  return (
    <section
      className={cn(
        "flex h-32 shrink-0 flex-col border-t bg-card",
        collapsed && "h-10",
      )}
    >
      <div className="flex h-10 shrink-0 items-center gap-2 border-b px-3 text-xs">
        <span className="font-semibold uppercase">{t("Runtime Console")}</span>
        <span className="text-muted-foreground" aria-hidden="true">
          ○
        </span>
        <span className="text-muted-foreground">
          {t("Waiting for the game Mod loader")}
        </span>
        <div className="ml-auto flex items-center gap-1">
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type="button"
                  className="flex size-7 items-center justify-center rounded hover:bg-muted"
                  aria-label={t("Copy")}
                  disabled
                />
              }
            >
              <Copy className="size-4" aria-hidden="true" />
            </TooltipTrigger>
            <TooltipContent>{t("Copy")}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type="button"
                  className="flex size-7 items-center justify-center rounded hover:bg-muted"
                  aria-label={t("Clear runtime console")}
                  disabled
                />
              }
            >
              <Clipboard className="size-4" aria-hidden="true" />
            </TooltipTrigger>
            <TooltipContent>{t("Clear runtime console")}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type="button"
                  className="flex size-7 items-center justify-center rounded hover:bg-muted"
                  aria-label={t(
                    collapsed
                      ? "Restore runtime console"
                      : "Minimize runtime console",
                  )}
                  onClick={onToggle}
                />
              }
            >
              <Minus
                className={cn("size-4", collapsed && "rotate-90")}
                aria-hidden="true"
              />
            </TooltipTrigger>
            <TooltipContent>
              {t(
                collapsed
                  ? "Restore runtime console"
                  : "Minimize runtime console",
              )}
            </TooltipContent>
          </Tooltip>
        </div>
      </div>
      {collapsed ? null : (
        <div className="p-3 text-xs text-muted-foreground">
          {t("Script logs and emitted IPC events appear here.")}
        </div>
      )}
    </section>
  );
}

function EditorPlaceholder({ loading }: { loading: boolean }) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="h-11 shrink-0 border-b" />
      <div className="h-10 shrink-0 border-b" />
      <div className="h-11 shrink-0 border-b" />
      <div className="min-h-0 flex-1 p-4">
        {loading ? <SourceLoading /> : null}
      </div>
      <div className="h-32 shrink-0 border-t" />
    </div>
  );
}

function SourceLoading() {
  return (
    <div className="flex flex-col gap-3" aria-label={t("Loading Mod source")}>
      {Array.from({ length: 12 }, (_, index) => (
        <Skeleton
          key={index}
          className="h-4"
          style={{ width: `${52 + ((index * 17) % 42)}%` }}
        />
      ))}
    </div>
  );
}

function extractCapabilityLabels(source: string): string[] {
  const labels: string[] = [];
  const pattern = /NTE_REQUIRES\("([^"]+)"\)/g;
  for (const match of source.matchAll(pattern)) {
    const label = match[1];
    if (!labels.includes(label)) {
      labels.push(label);
    }
    if (labels.length === 5) {
      break;
    }
  }
  return labels;
}

interface ErrorProps {
  error: ModStudioCommandError;
  onRetry: () => void | Promise<void>;
  compact?: boolean;
}

function WorkspaceError({ error, onRetry, compact = false }: ErrorProps) {
  return (
    <Alert variant="destructive">
      <TriangleAlert aria-hidden="true" />
      <AlertTitle>{t("Mod workspace unavailable")}</AlertTitle>
      {!compact ? (
        <AlertDescription>
          {tf(error.messageKey, error.messageArguments)}
        </AlertDescription>
      ) : null}
      <AlertAction>
        <Button variant="outline" size="sm" onClick={() => void onRetry()}>
          {t("Reload")}
        </Button>
      </AlertAction>
    </Alert>
  );
}

function DocumentError({ error, onRetry }: ErrorProps) {
  return (
    <Alert variant="destructive">
      <TriangleAlert aria-hidden="true" />
      <AlertTitle>{t("Mod source unavailable")}</AlertTitle>
      <AlertDescription>
        {tf(error.messageKey, error.messageArguments)}
      </AlertDescription>
      <AlertAction>
        <Button variant="outline" size="sm" onClick={() => void onRetry()}>
          {t("Reload")}
        </Button>
      </AlertAction>
    </Alert>
  );
}
