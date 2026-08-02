import { Search, X } from "lucide-react";
import { Activity, useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { DesktopTitlebar } from "@/components/nte/desktop-titlebar";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { CharacterDataPage } from "@/features/character-data/character-data-page";
import { DiagnosticsPage } from "@/features/diagnostics/diagnostics-page";
import { EmptyCurtainPage } from "@/features/empty-curtain/empty-curtain-page";
import { EncryptedIniPage } from "@/features/encrypted-ini/encrypted-ini-page";
import { HistoryPage } from "@/features/history/history-page";
import { ModStudioWorkspace } from "@/features/mod-studio/mod-studio-page";
import { PacketsPage } from "@/features/packets/packets-page";
import { ResourcesPage } from "@/features/resources/resources-page";
import { SettingsPage } from "@/features/settings/settings-page";
import { SkillsPage } from "@/features/skills/skills-page";
import { TimelinePage } from "@/features/timeline/timeline-page";
import { t, useTranslationRevision } from "@/lib/i18n";
import { applySettingsPresentation } from "@/lib/settings-presentation";
import { consoleControlClient } from "@/lib/tauri/console-control-client";
import { diagnosticsClient } from "@/lib/tauri/diagnostics-client";
import { historyClient } from "@/lib/tauri/history-client";
import { mainDpsClient } from "@/lib/tauri/main-dps-client";
import { parseMainDpsCommandError } from "@/lib/tauri/main-dps-contract";
import { settingsClient } from "@/lib/tauri/settings-client";
import type { InterfaceSettings } from "@/lib/tauri/settings-contract";

import { ConsoleCommandPalette } from "./console-command-palette";
import type { ConsoleCommandAction } from "./console-command-palette-model";
import {
  DEFAULT_CONSOLE_PAGE,
  adjacentConsolePage,
  consolePageActivityMode,
  isEditableKeyboardTarget,
  isConsolePageId,
  readConsoleSidebarCollapsed,
  resolveConsoleShortcut,
  type ConsolePageId,
  writeConsoleSidebarCollapsed,
} from "./console-navigation";
import { ConsoleSidebar } from "./console-sidebar";

export function ConsolePage() {
  useTranslationRevision();
  const [activePage, setActivePage] =
    useState<ConsolePageId>(DEFAULT_CONSOLE_PAGE);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(
    readConsoleSidebarCollapsed,
  );
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [commandError, setCommandError] = useState<string | null>(null);
  const [replayConfirmation, setReplayConfirmation] = useState<
    | { kind: "dialog"; replayKind: "json" | "pcapng" }
    | { kind: "path"; path: string }
    | null
  >(null);

  const navigateRelative = useCallback((offset: -1 | 1) => {
    setActivePage((current) => adjacentConsolePage(current, offset));
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<string>("console-navigate", (event) => {
      if (isConsolePageId(event.payload)) setActivePage(event.payload);
    }).then((next) => {
      unlisten = next;
    });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void mainDpsClient
      .subscribeReplayDrops((path) => {
        void importDroppedReplay(path, false);
      })
      .then((next) => {
        unlisten = next;
      });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const shortcut = resolveConsoleShortcut({
        key: event.key,
        ctrlKey: event.ctrlKey,
        altKey: event.altKey,
        shiftKey: event.shiftKey,
        repeat: event.repeat,
        editable: isEditableKeyboardTarget(event.target),
      });
      if (shortcut === null) return;
      event.preventDefault();
      if (shortcut === "command-palette")
        setCommandPaletteOpen((open) => !open);
      else navigateRelative(shortcut === "previous-page" ? -1 : 1);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [navigateRelative]);

  const toggleSidebar = () => {
    setSidebarCollapsed((collapsed) => {
      const next = !collapsed;
      writeConsoleSidebarCollapsed(next);
      return next;
    });
  };

  const executeCommand = useCallback(async (action: ConsoleCommandAction) => {
    setCommandError(null);
    try {
      switch (action.kind) {
        case "control":
          await consoleControlClient.execute(action.action);
          return;
        case "navigate":
          setActivePage(action.page);
          return;
        case "open-abyss":
          await settingsClient.openAbyssValues();
          return;
        case "open-hud-editor":
          await settingsClient.openHudEditor();
          return;
        case "import":
          try {
            await mainDpsClient.importReplay(action.format, false);
          } catch (error) {
            if (
              parseMainDpsCommandError(error).code === "confirmation_required"
            ) {
              setReplayConfirmation({
                kind: "dialog",
                replayKind: action.format,
              });
              return;
            }
            throw error;
          }
          return;
        case "export":
          if (action.format === "team") await settingsClient.exportTeamData();
          else if (action.format === "pcapng")
            await diagnosticsClient.exportPcapng();
          else await diagnosticsClient.exportJson();
          return;
        case "save-history":
          await historyClient.saveCurrent();
          return;
        case "layout":
          await settingsClient.applyLayoutProfile(action.profile);
          if (action.profile === "review") setActivePage("timeline");
          else if (action.profile === "research") setActivePage("packets");
          return;
        case "theme-preset":
          await updateInterface({ themePreset: action.value });
          return;
        case "accent":
          await updateInterface({ accent: action.value });
          return;
        case "density":
          await updateInterface({ density: action.value });
          return;
        case "toggle-theme": {
          const snapshot = await settingsClient.getSnapshot();
          await updateInterface(
            { darkMode: !snapshot.interface.darkMode },
            snapshot.interface,
          );
          return;
        }
        case "toggle-reduced-motion": {
          const snapshot = await settingsClient.getSnapshot();
          await updateInterface(
            {
              reduceMotion: !snapshot.interface.reduceMotion,
            },
            snapshot.interface,
          );
          return;
        }
        case "unavailable":
          return;
      }
    } catch (error) {
      setCommandError(commandErrorMessage(error));
    }
  }, []);

  return (
    <div className="desktop-window-shell">
      <DesktopTitlebar
        status={
          <button
            className="flex h-7 items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground transition-[color,background-color,transform] duration-150 ease-out hover:bg-muted hover:text-foreground active:scale-[0.97]"
            onClick={() => setCommandPaletteOpen(true)}
            title={`${t("Command palette")} · Ctrl+K`}
            type="button"
          >
            <Search className="size-3.5" aria-hidden="true" />
            <span>Ctrl+K</span>
          </button>
        }
        title={t("NTE Console")}
      />
      <main className="relative flex min-h-0 min-w-0 flex-1 overflow-hidden bg-background text-foreground select-none">
        <ConsoleSidebar
          activePage={activePage}
          collapsed={sidebarCollapsed}
          onNavigate={setActivePage}
          onToggle={toggleSidebar}
        />
        {commandError !== null && (
          <Alert
            className="ui-motion-alert absolute top-2 right-3 left-[calc(var(--spacing)*56)] z-20 shadow-md"
            variant="destructive"
          >
            <AlertDescription>{commandError}</AlertDescription>
            <button
              aria-label={t("Close")}
              className="absolute top-1.5 right-1.5 grid size-7 place-items-center rounded-md hover:bg-muted"
              onClick={() => setCommandError(null)}
              type="button"
            >
              <X className="size-3.5" aria-hidden="true" />
            </button>
          </Alert>
        )}
        <Activity
          name="console-timeline"
          mode={consolePageActivityMode(activePage, "timeline")}
        >
          <div
            className="console-page-stage flex min-h-0 min-w-0 flex-1"
            data-active={activePage === "timeline"}
          >
            <TimelinePage />
          </div>
        </Activity>
        <Activity
          name="console-skills"
          mode={consolePageActivityMode(activePage, "skills")}
        >
          <div
            className="console-page-stage flex min-h-0 min-w-0 flex-1"
            data-active={activePage === "skills"}
          >
            <SkillsPage />
          </div>
        </Activity>
        <Activity
          name="console-empty-curtain"
          mode={consolePageActivityMode(activePage, "empty-curtain")}
        >
          <div
            className="console-page-stage flex min-h-0 min-w-0 flex-1"
            data-active={activePage === "empty-curtain"}
          >
            <EmptyCurtainPage />
          </div>
        </Activity>
        <Activity
          name="console-character-data"
          mode={consolePageActivityMode(activePage, "character-data")}
        >
          <div
            className="console-page-stage flex min-h-0 min-w-0 flex-1"
            data-active={activePage === "character-data"}
          >
            <CharacterDataPage />
          </div>
        </Activity>
        <Activity
          name="console-encrypted-ini"
          mode={consolePageActivityMode(activePage, "encrypted-ini")}
        >
          <div
            className="console-page-stage flex min-h-0 min-w-0 flex-1"
            data-active={activePage === "encrypted-ini"}
          >
            <EncryptedIniPage />
          </div>
        </Activity>
        <Activity
          name="console-history"
          mode={consolePageActivityMode(activePage, "history")}
        >
          <div
            className="console-page-stage flex min-h-0 min-w-0 flex-1"
            data-active={activePage === "history"}
          >
            <HistoryPage />
          </div>
        </Activity>
        <Activity
          name="console-packets"
          mode={consolePageActivityMode(activePage, "packets")}
        >
          <div
            className="console-page-stage flex min-h-0 min-w-0 flex-1"
            data-active={activePage === "packets"}
          >
            <PacketsPage />
          </div>
        </Activity>
        <Activity
          name="console-settings"
          mode={consolePageActivityMode(activePage, "settings")}
        >
          <div
            className="console-page-stage flex min-h-0 min-w-0 flex-1"
            data-active={activePage === "settings"}
          >
            <SettingsPage />
          </div>
        </Activity>
        <Activity
          name="console-resources"
          mode={consolePageActivityMode(activePage, "resources")}
        >
          <div
            className="console-page-stage flex min-h-0 min-w-0 flex-1"
            data-active={activePage === "resources"}
          >
            <ResourcesPage />
          </div>
        </Activity>
        <Activity
          name="console-diagnostics"
          mode={consolePageActivityMode(activePage, "diagnostics")}
        >
          <div
            className="console-page-stage flex min-h-0 min-w-0 flex-1"
            data-active={activePage === "diagnostics"}
          >
            <DiagnosticsPage />
          </div>
        </Activity>
        <Activity
          name="console-mod-studio"
          mode={consolePageActivityMode(activePage, "mod-studio")}
        >
          <div
            className="console-page-stage flex min-h-0 min-w-0 flex-1"
            data-active={activePage === "mod-studio"}
          >
            <ModStudioWorkspace />
          </div>
        </Activity>
      </main>
      <ConsoleCommandPalette
        onExecute={executeCommand}
        onOpenChange={setCommandPaletteOpen}
        open={commandPaletteOpen}
      />
      {replayConfirmation !== null && (
        <div className="ui-motion-overlay fixed inset-0 z-[80] grid place-items-center bg-black/45 p-4">
          <section
            aria-labelledby="console-replay-confirm-title"
            aria-modal="true"
            className="ui-motion-dialog w-full max-w-md rounded-xl border bg-background p-5 shadow-2xl"
            role="dialog"
          >
            <h2
              id="console-replay-confirm-title"
              className="text-base font-semibold"
            >
              {t("Confirm Import")}
            </h2>
            <p className="mt-2 text-sm text-muted-foreground">
              {t(
                "Importing a replay stops the current task and clears existing stats.",
              )}
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <Button
                onClick={() => setReplayConfirmation(null)}
                variant="outline"
              >
                {t("Cancel")}
              </Button>
              <Button onClick={() => void confirmReplayImport()}>
                {t("Import")}
              </Button>
            </div>
          </section>
        </div>
      )}
    </div>
  );

  async function importDroppedReplay(path: string, replaceCurrent: boolean) {
    setCommandError(null);
    try {
      await mainDpsClient.importReplayPath(path, replaceCurrent);
    } catch (error) {
      const parsed = parseMainDpsCommandError(error);
      if (parsed.code === "confirmation_required" && !replaceCurrent) {
        setReplayConfirmation({ kind: "path", path });
        return;
      }
      setCommandError(commandErrorMessage(parsed));
    }
  }

  async function confirmReplayImport() {
    const pending = replayConfirmation;
    setReplayConfirmation(null);
    if (pending === null) return;
    if (pending.kind === "path") {
      await importDroppedReplay(pending.path, true);
      return;
    }
    setCommandError(null);
    try {
      await mainDpsClient.importReplay(pending.replayKind, true);
    } catch (error) {
      setCommandError(commandErrorMessage(parseMainDpsCommandError(error)));
    }
  }
}

async function updateInterface(
  patch: Partial<
    Pick<
      InterfaceSettings,
      "darkMode" | "themePreset" | "accent" | "density" | "reduceMotion"
    >
  >,
  existing?: InterfaceSettings,
): Promise<void> {
  const current = existing ?? (await settingsClient.getSnapshot()).interface;
  const snapshot = await settingsClient.setInterface({
    language: current.language,
    darkMode: patch.darkMode ?? current.darkMode,
    themePreset: patch.themePreset ?? current.themePreset,
    accent: patch.accent ?? current.accent,
    density: patch.density ?? current.density,
    reduceMotion: patch.reduceMotion ?? current.reduceMotion,
    islandNotifications: current.islandNotifications,
    islandOffsetX: current.islandOffsetX,
  });
  applySettingsPresentation(snapshot.interface);
}

function commandErrorMessage(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    if ("messageKey" in error && typeof error.messageKey === "string") {
      return t(error.messageKey);
    }
    if ("message" in error && typeof error.message === "string") {
      return error.message;
    }
  }
  return t("Operation failed");
}
