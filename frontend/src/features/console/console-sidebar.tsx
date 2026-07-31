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

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { t } from "@/lib/i18n";
import { cn } from "@/lib/utils";

import type { ConsolePageId } from "./console-navigation";

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
      { labelKey: "History", icon: History },
    ],
  },
  {
    labelKey: "Review",
    items: [
      { labelKey: "Timeline", icon: Timeline },
      { labelKey: "Skills", icon: Sparkles },
      { labelKey: "Console Loadout", icon: Backpack },
      { labelKey: "Mod Studio", icon: Puzzle, pageId: "mod-studio" },
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
                  active={item.pageId === activePage}
                  item={item}
                  key={item.labelKey}
                  labelHidden={collapsed}
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
        "flex h-9 w-full items-center gap-3 rounded-md px-2 text-sm text-muted-foreground max-[900px]:justify-center max-[900px]:px-0",
        "enabled:hover:bg-sidebar-accent enabled:hover:text-sidebar-accent-foreground disabled:cursor-default disabled:opacity-45",
        labelHidden && "justify-center px-0",
        active &&
          "bg-sidebar-primary text-sidebar-primary-foreground shadow-sm",
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
      <span
        className={cn("truncate max-[900px]:sr-only", labelHidden && "sr-only")}
      >
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
