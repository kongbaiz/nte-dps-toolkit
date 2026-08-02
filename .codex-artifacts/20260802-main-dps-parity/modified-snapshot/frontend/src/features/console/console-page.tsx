import { Activity, useState } from "react";

import { DesktopTitlebar } from "@/components/nte/desktop-titlebar";
import { CharacterDataPage } from "@/features/character-data/character-data-page";
import { DiagnosticsPage } from "@/features/diagnostics/diagnostics-page";
import { EncryptedIniPage } from "@/features/encrypted-ini/encrypted-ini-page";
import { ModStudioWorkspace } from "@/features/mod-studio/mod-studio-page";
import { PacketsPage } from "@/features/packets/packets-page";
import { ResourcesPage } from "@/features/resources/resources-page";
import { EmptyCurtainPage } from "@/features/empty-curtain/empty-curtain-page";
import { HistoryPage } from "@/features/history/history-page";
import { SettingsPage } from "@/features/settings/settings-page";
import { SkillsPage } from "@/features/skills/skills-page";
import { TimelinePage } from "@/features/timeline/timeline-page";
import { t, useTranslationRevision } from "@/lib/i18n";

import {
  consolePageActivityMode,
  type ConsolePageId,
} from "./console-navigation";
import { ConsoleSidebar } from "./console-sidebar";

export function ConsolePage() {
  useTranslationRevision();
  const [activePage, setActivePage] = useState<ConsolePageId>("mod-studio");
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);

  return (
    <div className="desktop-window-shell">
      <DesktopTitlebar title={t("NTE Console")} />
      <main className="flex min-h-0 min-w-0 flex-1 overflow-hidden bg-background text-foreground select-none">
        <ConsoleSidebar
          activePage={activePage}
          collapsed={sidebarCollapsed}
          onNavigate={setActivePage}
          onToggle={() => setSidebarCollapsed((collapsed) => !collapsed)}
        />
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
    </div>
  );
}
