import { lazy, Suspense, useMemo, useState } from "react";
import {
  Check,
  ChevronDown,
  ChevronLeft,
  Clipboard,
  Code2,
  Copy,
  FolderOpen,
  Minus,
  Radio,
  RefreshCw,
  TriangleAlert,
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
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { t, tf } from "@/lib/i18n";
import { useSettingsPresentation } from "@/lib/settings-presentation";
import type {
  ModStudioCommandError,
  ModStudioDocumentSummary,
} from "@/lib/tauri/mod-studio-contract";
import { MOD_STUDIO_MAX_SOURCE_BYTES } from "@/lib/tauri/mod-studio-contract";

import type { ModSourceCursor } from "./mod-source-editor";
import {
  formatRuntimeTimestamp,
  runtimeEntryPlainText,
  runtimeEntryText,
  visibleRuntimeEntries,
  type ModRuntimeConsoleFilter,
} from "./mod-runtime-console-model";

const ModSourceEditor = lazy(async () => {
  const module = await import("./mod-source-editor");
  return { default: module.ModSourceEditor };
});
import type {
  ModStudioRuntimeState,
  ModStudioSourceBuffer,
} from "./mod-studio-view-model";
import {
  useModStudio,
  type ModStudioEnableState,
  type ModStudioSaveState,
  type ModStudioSdkState,
} from "./use-mod-studio";

export function ModStudioWorkspace() {
  const {
    state,
    refresh,
    chooseDocument,
    selectedBuffer,
    selectedSaveState,
    selectedBufferDirty,
    dirtyDocumentIds,
    editSource,
    revertSource,
    saveSource,
    enableStates,
    setDocumentEnabled,
    runtimeState,
    sdkState,
  } = useModStudio();
  const [consoleCollapsed, setConsoleCollapsed] = useState(false);

  return (
    <section className="flex min-w-0 flex-1 flex-col p-3">
      <ModStudioHeader runtimeState={runtimeState} onRefresh={refresh} />
      <div className="mt-3 grid min-h-0 flex-1 grid-cols-[clamp(12rem,21vw,17rem)_minmax(0,1fr)] overflow-hidden border bg-card max-[900px]:grid-cols-[12rem_minmax(0,1fr)]">
        <ExplorerPane
          state={state}
          selectedId={state.status === "ready" ? state.selectedId : null}
          onSelect={chooseDocument}
          onRefresh={refresh}
          dirtyDocumentIds={dirtyDocumentIds}
          enableStates={enableStates}
          onSetEnabled={(id, enabled) => {
            void setDocumentEnabled(id, enabled);
          }}
        />
        <section className="flex min-h-0 min-w-0 flex-col">
          {state.status === "ready" ? (
            <EditorPane
              selectedId={state.selectedId}
              document={state.document}
              sourceBuffer={selectedBuffer}
              saveState={selectedSaveState}
              dirty={selectedBufferDirty}
              onSourceChange={(source) => {
                if (selectedBuffer !== null) {
                  editSource(
                    state.selectedId,
                    selectedBuffer.savedSource,
                    source,
                  );
                }
              }}
              onSave={() => {
                if (selectedBuffer !== null && selectedBufferDirty) {
                  void saveSource(state.selectedId, selectedBuffer);
                }
              }}
              onRevert={() => {
                if (selectedBuffer !== null) {
                  revertSource(state.selectedId, selectedBuffer);
                }
              }}
              onRetry={refresh}
              consoleCollapsed={consoleCollapsed}
              runtimeState={runtimeState}
              sdkState={sdkState}
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
  );
}

function ModStudioHeader({
  runtimeState,
  onRefresh,
}: {
  runtimeState: ModStudioRuntimeState;
  onRefresh: () => void | Promise<void>;
}) {
  const runtimeConnected = runtimeState.connection === "connected";
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
        <Radio
          className={cn(
            "size-4",
            runtimeConnected
              ? "text-[var(--console-success)]"
              : "text-[var(--console-warning)]",
          )}
          aria-hidden="true"
        />
        <span className="font-medium">{t("In-game Mod loader")}</span>
        <span className="text-muted-foreground">
          {t(
            runtimeConnected
              ? "Hot reload connected"
              : "Waiting for the game Mod loader",
          )}
        </span>
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
  dirtyDocumentIds: Set<string>;
  enableStates: Record<string, ModStudioEnableState>;
  onSetEnabled: (id: string, enabled: boolean) => void;
}

function ExplorerPane({
  state,
  selectedId,
  onSelect,
  onRefresh,
  dirtyDocumentIds,
  enableStates,
  onSetEnabled,
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
                dirty={dirtyDocumentIds.has(document.id)}
                enableState={enableStates[document.id] ?? { status: "idle" }}
                onSelect={() => onSelect(document.id)}
                onSetEnabled={(enabled) => onSetEnabled(document.id, enabled)}
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
  dirty: boolean;
  enableState: ModStudioEnableState;
  onSelect: () => void;
  onSetEnabled: (enabled: boolean) => void;
}

function DocumentItem({
  document,
  selected,
  dirty,
  enableState,
  onSelect,
  onSetEnabled,
}: DocumentItemProps) {
  const enableError =
    enableState.status === "error"
      ? tf(enableState.error.messageKey, enableState.error.messageArguments)
      : null;
  const switchLabel = t(
    document.enabled ? "Disable this Mod" : "Enable this Mod",
  );
  return (
    <div
      className={cn(
        "relative flex h-10 w-full items-center gap-2 px-3 text-left text-sm hover:bg-muted",
        selected && "bg-muted",
      )}
    >
      {selected ? (
        <span
          className="absolute inset-y-0 left-0 w-0.5 bg-foreground"
          aria-hidden="true"
        />
      ) : null}
      <button
        type="button"
        aria-current={selected ? "page" : undefined}
        className="flex min-w-0 flex-1 items-center gap-2 self-stretch text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
        onClick={onSelect}
      >
        <Code2
          className="size-4 shrink-0 text-muted-foreground"
          aria-hidden="true"
        />
        <span className="min-w-0 flex-1 truncate font-mono">
          {document.id}.nte
        </span>
        {dirty ? (
          <span
            className="text-muted-foreground"
            aria-label={t("Unsaved changes")}
          >
            ●
          </span>
        ) : null}
        <span className="sr-only">
          {tf("{0} lines · {1} bytes", [
            document.lineCount.toString(),
            document.sourceBytes.toString(),
          ])}
        </span>
      </button>
      {enableError !== null ? (
        <Tooltip>
          <TooltipTrigger
            render={
              <span
                className="flex size-6 items-center justify-center text-destructive"
                aria-label={enableError}
              />
            }
          >
            <TriangleAlert className="size-4" aria-hidden="true" />
          </TooltipTrigger>
          <TooltipContent>{enableError}</TooltipContent>
        </Tooltip>
      ) : null}
      <span
        title={
          dirty
            ? t("Save or revert changes before changing Mod enablement.")
            : switchLabel
        }
      >
        <Switch
          size="sm"
          checked={document.enabled}
          disabled={dirty || enableState.status === "saving"}
          aria-label={switchLabel}
          onCheckedChange={onSetEnabled}
        />
      </span>
    </div>
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
  document: ReadyState["document"];
  sourceBuffer: ModStudioSourceBuffer | null;
  saveState: ModStudioSaveState;
  dirty: boolean;
  onSourceChange: (source: string) => void;
  onSave: () => void;
  onRevert: () => void;
  onRetry: () => void | Promise<void>;
  consoleCollapsed: boolean;
  runtimeState: ModStudioRuntimeState;
  sdkState: ModStudioSdkState;
  onToggleConsole: () => void;
}

function EditorPane({
  selectedId,
  document,
  sourceBuffer,
  saveState,
  dirty,
  onSourceChange,
  onSave,
  onRevert,
  onRetry,
  consoleCollapsed,
  runtimeState,
  sdkState,
  onToggleConsole,
}: EditorPaneProps) {
  return (
    <>
      <div className="flex h-11 shrink-0 items-center gap-2 border-b px-3">
        <Code2 className="size-4 text-muted-foreground" aria-hidden="true" />
        <span className="font-mono text-sm">{selectedId}.nte</span>
        {dirty ? (
          <span
            className="text-muted-foreground"
            aria-label={t("Unsaved changes")}
          >
            ●
          </span>
        ) : null}
        <div className="ml-auto flex gap-2">
          <Button
            variant="outline"
            size="sm"
            className="h-8"
            disabled={!dirty || saveState.status === "saving"}
            onClick={onSave}
          >
            {t(saveState.status === "saving" ? "Saving Mod..." : "Save Mod")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            className="h-8"
            disabled={!dirty || saveState.status === "saving"}
            onClick={onRevert}
          >
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
        {document.status === "ready" && sourceBuffer !== null
          ? extractCapabilityLabels(sourceBuffer.source).map((label) => (
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
            documentId={selectedId}
            source={sourceBuffer?.source ?? document.document.source}
            dirty={dirty}
            saveState={saveState}
            sdkState={sdkState}
            onChange={onSourceChange}
            onSave={onSave}
          />
        ) : null}
      </div>
      <RuntimeConsole
        collapsed={consoleCollapsed}
        runtimeState={runtimeState}
        onToggle={onToggleConsole}
      />
    </>
  );
}

function SourceEditor({
  documentId,
  source,
  dirty,
  saveState,
  sdkState,
  onChange,
  onSave,
}: {
  documentId: string;
  source: string;
  dirty: boolean;
  saveState: ModStudioSaveState;
  sdkState: ModStudioSdkState;
  onChange: (source: string) => void;
  onSave: () => void;
}) {
  const [cursor, setCursor] = useState<ModSourceCursor>({
    line: 1,
    column: 1,
  });
  const editorTheme = useSettingsPresentation().darkMode ? "dark" : "light";
  const currentSourceBytes = new TextEncoder().encode(source).length;
  const diagnosticLine =
    saveState.status === "error" ? saveState.error.diagnosticLine : null;
  const statusText =
    saveState.status === "error"
      ? tf(saveState.error.messageKey, saveState.error.messageArguments)
      : saveState.status === "saving"
        ? t("Saving Mod...")
        : saveState.status === "saved" && !dirty
          ? t("Mod saved")
          : dirty
            ? t("Unsaved changes")
            : t("Saved");

  return (
    <div
      className="mod-source-editor-shell flex min-h-0 flex-1 flex-col bg-[var(--editor-background)]"
      data-editor-theme={editorTheme}
    >
      <Suspense
        fallback={
          <div
            className="min-h-0 flex-1 bg-[var(--editor-background)]"
            role="status"
            aria-label={t("Loading Mod source")}
          />
        }
      >
        <ModSourceEditor
          key={documentId}
          source={source}
          diagnosticLine={diagnosticLine}
          label={t("Mod source editor")}
          sdkSchema={sdkState.status === "ready" ? sdkState.schema : null}
          theme={editorTheme}
          onChange={onChange}
          onCursorChange={setCursor}
          onSave={onSave}
        />
      </Suspense>
      <div
        className="flex h-7 shrink-0 items-center bg-[var(--editor-status-background)] px-2 text-[11px] text-[var(--editor-status-foreground)]"
        aria-live="polite"
      >
        {saveState.status === "error" ? (
          <TriangleAlert className="mr-1 size-3.5" aria-hidden="true" />
        ) : (
          <Check className="mr-1 size-3.5" aria-hidden="true" />
        )}
        <span className="truncate">{statusText}</span>
        <div className="ml-auto flex h-full items-center divide-x divide-[var(--editor-status-border)]">
          <span className="px-3">
            {t(editorTheme === "light" ? "Light" : "Dark")}
          </span>
          <span className="px-3">
            {tf("Ln {0}, Col {1}", [
              cursor.line.toString(),
              cursor.column.toString(),
            ])}
          </span>
          <span className="px-3">
            {tf("{0} / {1} bytes", [
              currentSourceBytes.toString(),
              MOD_STUDIO_MAX_SOURCE_BYTES.toString(),
            ])}
          </span>
          <span className="px-3">{t("Ctrl+S")}</span>
          <span className="pl-3">
            {sdkState.status === "ready"
              ? tf("NTE C++ API v{0}", [
                  sdkState.schema.schemaVersion.toString(),
                ])
              : t(
                  sdkState.status === "loading"
                    ? "Loading API schema"
                    : "API schema unavailable",
                )}
          </span>
        </div>
      </div>
    </div>
  );
}

function RuntimeConsole({
  collapsed,
  runtimeState,
  onToggle,
}: {
  collapsed: boolean;
  runtimeState: ModStudioRuntimeState;
  onToggle: () => void;
}) {
  const [filter, setFilter] = useState<ModRuntimeConsoleFilter>("all");
  const [cleared, setCleared] = useState<{
    generation: string | null;
    throughSequence: string | null;
  }>({ generation: null, throughSequence: null });
  const [copyStatus, setCopyStatus] = useState<"idle" | "copied" | "error">(
    "idle",
  );
  const connected = runtimeState.connection === "connected";
  const connectionText =
    runtimeState.error !== null
      ? tf(runtimeState.error.messageKey, runtimeState.error.messageArguments)
      : t(
          connected
            ? "Hot reload connected"
            : "Waiting for the game Mod loader",
        );
  const entries = useMemo(
    () =>
      visibleRuntimeEntries(
        runtimeState.entries,
        runtimeState.generation,
        cleared,
        filter,
      ),
    [cleared, filter, runtimeState.entries, runtimeState.generation],
  );
  const copyEntries = async () => {
    try {
      await navigator.clipboard.writeText(
        entries.map((entry) => runtimeEntryPlainText(entry, tf)).join("\n"),
      );
      setCopyStatus("copied");
    } catch {
      setCopyStatus("error");
    }
  };
  const clearEntries = () => {
    setCleared({
      generation: runtimeState.generation,
      throughSequence: runtimeState.entries.at(-1)?.sequence ?? null,
    });
    setCopyStatus("idle");
  };
  return (
    <section
      className={cn(
        "flex h-32 shrink-0 flex-col border-t bg-card",
        collapsed && "h-10",
      )}
    >
      <div className="flex h-10 shrink-0 items-center gap-2 border-b px-3 text-xs">
        <span className="font-semibold uppercase">{t("Runtime Console")}</span>
        <span
          className={cn(
            connected
              ? "text-[var(--console-success)]"
              : "text-muted-foreground",
          )}
          aria-hidden="true"
        >
          ●
        </span>
        <span className="truncate text-muted-foreground" aria-live="polite">
          {connectionText}
        </span>
        <div className="ml-auto flex items-center gap-1">
          <label className="sr-only" htmlFor="mod-runtime-filter">
            {t("Filter runtime console")}
          </label>
          <select
            id="mod-runtime-filter"
            className="h-7 rounded border bg-background px-1.5 text-[11px]"
            value={filter}
            onChange={(event) =>
              setFilter(event.currentTarget.value as ModRuntimeConsoleFilter)
            }
          >
            <option value="all">{t("All runtime entries")}</option>
            <option value="log">{t("Logs")}</option>
            <option value="event">{t("Events")}</option>
            <option value="info">{t("Info")}</option>
            <option value="warning">{t("Warnings")}</option>
            <option value="error">{t("Errors")}</option>
          </select>
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type="button"
                  className="flex size-7 items-center justify-center rounded hover:bg-muted"
                  aria-label={t("Copy")}
                  disabled={entries.length === 0}
                  onClick={() => void copyEntries()}
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
                  disabled={runtimeState.entries.length === 0}
                  onClick={clearEntries}
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
      <span className="sr-only" aria-live="polite">
        {copyStatus === "copied"
          ? t("Runtime console copied")
          : copyStatus === "error"
            ? t("Failed to copy runtime console")
            : ""}
      </span>
      {collapsed ? null : (
        <div
          className="min-h-0 flex-1 overflow-y-auto px-3 py-2 font-mono text-[11px]"
          role="log"
          aria-label={t("Hot reload status")}
        >
          {entries.length === 0 ? (
            <p className="font-sans text-xs text-muted-foreground">
              {t(
                runtimeState.entries.length > 0
                  ? "No runtime entries match the current filter."
                  : connected
                    ? "Runtime connected; waiting for source or enabled-set changes."
                    : "Script logs and emitted IPC events appear here.",
              )}
            </p>
          ) : (
            entries.map((entry) => (
              <div
                className="flex min-w-0 items-start gap-2 leading-5"
                key={`${runtimeState.generation}-${entry.sequence}`}
              >
                <span
                  className={cn(
                    "w-12 shrink-0",
                    entry.kind === "log" &&
                      entry.level === "error" &&
                      "text-destructive",
                    entry.kind === "log" &&
                      entry.level === "warning" &&
                      "text-[var(--console-warning)]",
                    (entry.kind === "event" || entry.level === "info") &&
                      "text-muted-foreground",
                  )}
                >
                  [
                  {entry.kind === "event"
                    ? "EVENT"
                    : entry.level === "warning"
                      ? "WARN"
                      : entry.level.toUpperCase()}
                  ]
                </span>
                <span className="shrink-0 text-muted-foreground">
                  {formatRuntimeTimestamp(entry.timestamp100ns)}
                </span>
                <span className="shrink-0 text-muted-foreground">
                  [{entry.modId}]
                </span>
                <span className="min-w-0 break-words">
                  {runtimeEntryText(entry, tf)}
                </span>
              </div>
            ))
          )}
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
