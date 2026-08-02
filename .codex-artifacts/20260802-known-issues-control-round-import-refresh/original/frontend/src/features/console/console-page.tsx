import { Search, X } from "lucide-react";
import { Activity, useCallback, useEffect, useState } from "react";

import { DesktopTitlebar } from "@/components/nte/desktop-titlebar";
import { Alert, AlertDescription } from "@/components/ui/alert";
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
import { diagnosticsClient } from "@/lib/tauri/diagnostics-client";
import { historyClient } from "@/lib/tauri/history-client";
import { settingsClient } from "@/lib/tauri/settings-client";
import type { InterfaceSettings } from "@/lib/tauri/settings-contract";

import { ConsoleCommandPalette } from "./console-command-palette";
import type { ConsoleCommandAction } from "./console-command-palette-model";
import {
  DEFAULT_CONSOLE_PAGE,
  adjacentConsolePage,
  consolePageActivityMode,
  isEditableKeyboardTarget,
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

  const navigateRelative = useCallback((offset: -1 | 1) => {
    setActivePage((current) => adjacentConsolePage(current, offset));
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
          if (action.format === "pcapng")
            await diagnosticsClient.importPcapng();
          else await diagnosticsClient.importJson();
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
            className="flex h-7 items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
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
            className="absolute top-2 right-3 left-[calc(var(--spacing)*56)] z-20 shadow-md"
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
          <div className="flex min-h-0 min-w-0 flex-1">
            <TimelinePage />
          </div>
        </Activity>
        <Activity
          name="console-skills"
          mode={consolePageActivityMode(activePage, "skills")}
        >
          <div className="flex min-h-0 min-w-0 flex-1">
            <SkillsPage />
          </div>
        </Activity>
        <Activity
          name="console-empty-curtain"
          mode={consolePageActivityMode(activePage, "empty-curtain")}
        >
          <div className="flex min-h-0 min-w-0 flex-1">
            <EmptyCurtainPage />
          </div>
        </Activity>
        <Activity
          name="console-character-data"
          mode={consolePageActivityMode(activePage, "character-data")}
        >
          <div className="flex min-h-0 min-w-0 flex-1">
            <CharacterDataPage />
          </div>
        </Activity>
        <Activity
          name="console-encrypted-ini"
          mode={consolePageActivityMode(activePage, "encrypted-ini")}
        >
          <div className="flex min-h-0 min-w-0 flex-1">
            <EncryptedIniPage />
          </div>
        </Activity>
        <Activity
          name="console-history"
          mode={consolePageActivityMode(activePage, "history")}
        >
          <div className="flex min-h-0 min-w-0 flex-1">
            <HistoryPage />
          </div>
        </Activity>
        <Activity
          name="console-packets"
          mode={consolePageActivityMode(activePage, "packets")}
        >
          <div className="flex min-h-0 min-w-0 flex-1">
            <PacketsPage />
          </div>
        </Activity>
        <Activity
          name="console-settings"
          mode={consolePageActivityMode(activePage, "settings")}
        >
          <div className="flex min-h-0 min-w-0 flex-1">
            <SettingsPage />
          </div>
        </Activity>
        <Activity
          name="console-resources"
          mode={consolePageActivityMode(activePage, "resources")}
        >
          <div className="flex min-h-0 min-w-0 flex-1">
            <ResourcesPage />
          </div>
        </Activity>
        <Activity
          name="console-diagnostics"
          mode={consolePageActivityMode(activePage, "diagnostics")}
        >
          <div className="flex min-h-0 min-w-0 flex-1">
            <DiagnosticsPage />
          </div>
        </Activity>
        <Activity
          name="console-mod-studio"
          mode={consolePageActivityMode(activePage, "mod-studio")}
        >
          <div className="flex min-h-0 min-w-0 flex-1">
            <ModStudioWorkspace />
          </div>
        </Activity>
      </main>
      <ConsoleCommandPalette
        onExecute={executeCommand}
        onOpenChange={setCommandPaletteOpen}
        open={commandPaletteOpen}
      />
    </div>
  );
}

async function updateInterface(
  patch: Partial<
    Pick<
      InterfaceSettings,
      "themePreset" | "accent" | "density" | "reduceMotion"
    >
  >,
  existing?: InterfaceSettings,
): Promise<void> {
  const current = existing ?? (await settingsClient.getSnapshot()).interface;
  await settingsClient.setInterface({
    language: current.language,
    themePreset: patch.themePreset ?? current.themePreset,
    accent: patch.accent ?? current.accent,
    density: patch.density ?? current.density,
    reduceMotion: patch.reduceMotion ?? current.reduceMotion,
    islandNotifications: current.islandNotifications,
    islandOffsetX: current.islandOffsetX,
  });
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
