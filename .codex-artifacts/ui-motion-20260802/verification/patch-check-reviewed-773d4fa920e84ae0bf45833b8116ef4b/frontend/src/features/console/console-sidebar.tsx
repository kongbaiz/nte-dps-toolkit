import {
  Activity,
  Backpack,
  ChevronLeft,
  ChevronsLeft,
  Folder,
  History,
  LockKeyhole,
  Puzzle,
  Radio,
  Settings,
  Sparkles,
  Timeline,
  UserRound,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useState } from "react";

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { t } from "@/lib/i18n";
import { cn } from "@/lib/utils";

import type { ConsolePageId } from "./console-navigation";
import {
  CONSOLE_SIDEBAR_COLLAPSE_QUERY,
  consoleSidebarRowClasses,
  resolveConsoleSidebarPresentation,
} from "./console-sidebar-model";

interface ConsoleNavItem {
  labelKey: string;
  icon: LucideIcon;
  pageId?: ConsolePageId;
}

const CONSOLE_NAV_GROUPS: Array<{
  labelKey: string;
  items: ConsoleNavItem[];
}> = [
  {
    labelKey: "Common",
    items: [
      { labelKey: "Settings", icon: Settings, pageId: "settings" },
      { labelKey: "History", icon: History, pageId: "history" },
    ],
  },
  {
    labelKey: "Review",
    items: [
      { labelKey: "Timeline", icon: Timeline, pageId: "timeline" },
      { labelKey: "Skills", icon: Sparkles, pageId: "skills" },
      {
        labelKey: "Console Loadout",
        icon: Backpack,
        pageId: "empty-curtain",
      },
      { labelKey: "Mod Studio", icon: Puzzle, pageId: "mod-studio" },
    ],
  },
  {
    labelKey: "Advanced",
    items: [
      {
        labelKey: "Character Data",
        icon: UserRound,
        pageId: "character-data",
      },
      {
        labelKey: "Encrypted INI",
        icon: LockKeyhole,
        pageId: "encrypted-ini",
      },
      { labelKey: "Packets", icon: Radio, pageId: "packets" },
      { labelKey: "Resources", icon: Folder, pageId: "resources" },
      { labelKey: "Diagnostics", icon: Activity, pageId: "diagnostics" },
    ],
  },
];

interface ConsoleSidebarProps {
  activePage: ConsolePageId;
  collapsed: boolean;
  onNavigate: (page: ConsolePageId) => void;
  onToggle: () => void;
}

export function ConsoleSidebar({
  activePage,
  collapsed,
  onNavigate,
  onToggle,
}: ConsoleSidebarProps) {
  const automaticallyCollapsed = useConsoleSidebarAutoCollapsed();
  const presentation = resolveConsoleSidebarPresentation(
    collapsed,
    automaticallyCollapsed,
  );

  return (
    <aside
      className={cn(
        "flex w-52 shrink-0 flex-col border-r bg-sidebar px-3 py-3 text-sidebar-foreground transition-[width] duration-150",
        presentation.collapsed && "w-14 px-1.5",
      )}
      aria-label={t("Console navigation")}
    >
      {presentation.allowToggle && (
        <Tooltip>
          <TooltipTrigger
            render={
              <button
                type="button"
                className="mb-2 flex h-9 w-full items-center justify-center gap-2 rounded-md border bg-card text-sm text-muted-foreground hover:bg-muted"
                aria-label={t(
                  presentation.collapsed
                    ? "Expand sidebar"
                    : "Collapse sidebar",
                )}
                onClick={onToggle}
              />
            }
          >
            {presentation.collapsed ? (
              <ChevronLeft className="size-4 rotate-180" aria-hidden="true" />
            ) : (
              <>
                <ChevronsLeft className="size-4" aria-hidden="true" />
                <span>{t("Collapse")}</span>
              </>
            )}
          </TooltipTrigger>
          <TooltipContent>
            {t(presentation.collapsed ? "Expand sidebar" : "Collapse sidebar")}
          </TooltipContent>
        </Tooltip>
      )}

      <nav className="min-h-0 overflow-y-auto">
        {CONSOLE_NAV_GROUPS.map((group) => (
          <div className="mb-3" key={group.labelKey}>
            <p
              className={cn(
                "mb-1 px-2 text-[11px] text-muted-foreground",
                presentation.collapsed && "sr-only",
              )}
            >
              {t(group.labelKey)}
            </p>
            <div className="flex flex-col gap-0.5">
              {group.items.map((item) => (
                <ConsoleNavRow
                  active={item.pageId === activePage}
                  item={item}
                  key={item.labelKey}
                  labelHidden={presentation.collapsed}
                  onNavigate={onNavigate}
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
  active,
  item,
  labelHidden,
  onNavigate,
}: {
  active: boolean;
  item: ConsoleNavItem;
  labelHidden: boolean;
  onNavigate: (page: ConsolePageId) => void;
}) {
  const Icon = item.icon;
  const row = (
    <button
      type="button"
      className={cn(
        "flex h-9 w-full items-center gap-3 rounded-md px-2 text-sm",
        labelHidden && "justify-center px-0",
        consoleSidebarRowClasses(active, item.pageId === undefined),
      )}
      aria-current={active ? "page" : undefined}
      disabled={item.pageId === undefined}
      onClick={() => {
        if (item.pageId !== undefined) {
          onNavigate(item.pageId);
        }
      }}
    >
      <Icon className="size-[18px] shrink-0" aria-hidden="true" />
      <span className={cn("truncate", labelHidden && "sr-only")}>
        {t(item.labelKey)}
      </span>
    </button>
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

function useConsoleSidebarAutoCollapsed(): boolean {
  const [collapsed, setCollapsed] = useState(() => {
    if (typeof window.matchMedia !== "function") return false;
    return window.matchMedia(CONSOLE_SIDEBAR_COLLAPSE_QUERY).matches;
  });

  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const mediaQuery = window.matchMedia(CONSOLE_SIDEBAR_COLLAPSE_QUERY);
    const update = () => setCollapsed(mediaQuery.matches);
    update();
    mediaQuery.addEventListener("change", update);
    return () => mediaQuery.removeEventListener("change", update);
  }, []);

  return collapsed;
}
