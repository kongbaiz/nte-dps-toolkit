import { useState } from "react";

import { ModStudioWorkspace } from "@/features/mod-studio/mod-studio-page";
import { SettingsPage } from "@/features/settings/settings-page";
import { cn } from "@/lib/utils";
import { useTranslationRevision } from "@/lib/i18n";
import { useSettingsPresentation } from "@/lib/settings-presentation";

import type { ConsolePageId } from "./console-navigation";
import { ConsoleSidebar } from "./console-sidebar";

export function ConsolePage() {
  useTranslationRevision();
  const presentation = useSettingsPresentation();
  const [activePage, setActivePage] = useState<ConsolePageId>("mod-studio");
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);

  return (
    <main
      className={cn(
        "flex h-screen min-h-0 w-screen overflow-hidden bg-background text-foreground select-none",
        !presentation.darkMode && "console-light",
      )}
    >
      <ConsoleSidebar
        activePage={activePage}
        collapsed={sidebarCollapsed}
        onNavigate={setActivePage}
        onToggle={() => setSidebarCollapsed((collapsed) => !collapsed)}
      />
      <div
        className={cn(
          "min-h-0 min-w-0 flex-1",
          activePage === "settings" ? "flex" : "hidden",
        )}
        aria-hidden={activePage !== "settings"}
      >
        <SettingsPage />
      </div>
      <div
        className={cn(
          "min-h-0 min-w-0 flex-1",
          activePage === "mod-studio" ? "flex" : "hidden",
        )}
        aria-hidden={activePage !== "mod-studio"}
      >
        <ModStudioWorkspace />
      </div>
    </main>
  );
}
