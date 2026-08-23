import {
  Activity,
  Backpack,
  ChevronLeft,
  ChevronsLeft,
  History,
  Keyboard,
  LockKeyhole,
  Puzzle,
  Radio,
  Settings,
  Sparkles,
  Star,
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
  pageId: ConsolePageId;
}

const CONSOLE_NAV_ITEMS: Record<ConsolePageId, ConsoleNavItem> = {
  settings: { labelKey: "Settings", icon: Settings, pageId: "settings" },
  shortcuts: {
    labelKey: "Shortcuts",
    icon: Keyboard,
    pageId: "shortcuts",
  },
  history: { labelKey: "History", icon: History, pageId: "history" },
  timeline: { labelKey: "Timeline", icon: Timeline, pageId: "timeline" },
  skills: { labelKey: "Skills", icon: Sparkles, pageId: "skills" },
  "empty-curtain": {
    labelKey: "Console Loadout",
    icon: Backpack,
    pageId: "empty-curtain",
  },
  "mod-studio": {
    labelKey: "Mod Studio",
    icon: Puzzle,
    pageId: "mod-studio",
  },
  "character-data": {
    labelKey: "Character Data",
    icon: UserRound,
    pageId: "character-data",
  },
  "encrypted-ini": {
    labelKey: "Encrypted INI",
    icon: LockKeyhole,
    pageId: "encrypted-ini",
  },
  packets: { labelKey: "Packets", icon: Radio, pageId: "packets" },
  diagnostics: {
    labelKey: "Diagnostics",
    icon: Activity,
    pageId: "diagnostics",
  },
};

const CONSOLE_NAV_GROUPS: Array<{
  labelKey: string;
  pages: ConsolePageId[];
}> = [
  {
    labelKey: "Review",
    pages: ["history", "timeline", "skills", "empty-curtain", "mod-studio"],
  },
  {
    labelKey: "Advanced",
    pages: [
      "settings",
      "shortcuts",
      "character-data",
      "encrypted-ini",
      "packets",
      "diagnostics",
    ],
  },
];

interface ConsoleSidebarProps {
  activePage: ConsolePageId;
  collapsed: boolean;
  favoritePages: readonly ConsolePageId[];
  onFavoriteChange: (page: ConsolePageId, favorite: boolean) => void;
  onNavigate: (page: ConsolePageId) => void;
  onToggle: () => void;
}

export function ConsoleSidebar({
  activePage,
  collapsed,
  favoritePages,
  onFavoriteChange,
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
        "flex w-52 shrink-0 flex-col border-r bg-sidebar px-3 py-3 text-sidebar-foreground transition-[width,padding] duration-[var(--motion-duration-slow)] [transition-timing-function:var(--motion-ease-emphasized)]",
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
                className="mb-2 flex h-9 w-full items-center justify-center gap-2 rounded-md border bg-card text-sm text-muted-foreground transition-[color,background-color,border-color,transform] duration-150 ease-out hover:bg-muted active:scale-[0.97]"
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

      <nav className="console-sidebar-scroll min-h-0 overflow-x-hidden overflow-y-auto overscroll-contain">
        <div className="mb-3">
          <p
            className={cn(
              "mb-1 px-2 text-[11px] text-muted-foreground",
              presentation.collapsed && "sr-only",
            )}
          >
            {t("Common")}
          </p>
          <div className="flex flex-col gap-0.5">
            {favoritePages.length === 0 && !presentation.collapsed ? (
              <p className="px-2 py-1 text-xs leading-relaxed text-muted-foreground">
                {t("No favorites yet")}
              </p>
            ) : null}
            {favoritePages.map((page) => (
              <ConsoleNavRow
                active={page === activePage}
                favorite
                item={CONSOLE_NAV_ITEMS[page]}
                key={page}
                labelHidden={presentation.collapsed}
                onFavoriteChange={onFavoriteChange}
                onNavigate={onNavigate}
              />
            ))}
          </div>
        </div>
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
              {group.pages
                .filter((page) => !favoritePages.includes(page))
                .map((page) => (
                  <ConsoleNavRow
                    active={page === activePage}
                    favorite={false}
                    item={CONSOLE_NAV_ITEMS[page]}
                    key={page}
                    labelHidden={presentation.collapsed}
                    onFavoriteChange={onFavoriteChange}
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
  favorite,
  item,
  labelHidden,
  onFavoriteChange,
  onNavigate,
}: {
  active: boolean;
  favorite: boolean;
  item: ConsoleNavItem;
  labelHidden: boolean;
  onFavoriteChange: (page: ConsolePageId, favorite: boolean) => void;
  onNavigate: (page: ConsolePageId) => void;
}) {
  const Icon = item.icon;
  const navigationButton = (
    <button
      type="button"
      className={cn(
        "console-nav-row group/nav-row flex h-9 w-full items-center gap-3 rounded-md px-2 text-sm",
        labelHidden && "justify-center px-0",
        consoleSidebarRowClasses(active, false),
      )}
      aria-current={active ? "page" : undefined}
      onClick={() => onNavigate(item.pageId)}
    >
      <span className="grid size-[18px] shrink-0 place-items-center transition-transform duration-200 ease-out group-hover/nav-row:scale-110 group-active/nav-row:scale-95">
        <Icon className="size-[18px]" aria-hidden="true" />
      </span>
      <span className={cn("truncate", labelHidden && "sr-only")}>
        {t(item.labelKey)}
      </span>
    </button>
  );

  if (labelHidden) {
    return (
      <Tooltip>
        <TooltipTrigger render={navigationButton} />
        <TooltipContent side="right">{t(item.labelKey)}</TooltipContent>
      </Tooltip>
    );
  }
  return (
    <div className="group/favorite-row flex items-center gap-1">
      <div className="min-w-0 flex-1">{navigationButton}</div>
      <Tooltip>
        <TooltipTrigger
          render={
            <button
              type="button"
              className={cn(
                "grid size-8 shrink-0 place-items-center rounded-md text-muted-foreground opacity-0 transition-[opacity,color,background-color] hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover/favorite-row:opacity-100",
                favorite && "text-amber-500 opacity-100",
              )}
              aria-label={t(
                favorite ? "Remove from favorites" : "Add to favorites",
              )}
              onClick={() => onFavoriteChange(item.pageId, !favorite)}
            />
          }
        >
          <Star className={cn("size-4", favorite && "fill-current")} />
        </TooltipTrigger>
        <TooltipContent>
          {t(favorite ? "Remove from favorites" : "Add to favorites")}
        </TooltipContent>
      </Tooltip>
    </div>
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
