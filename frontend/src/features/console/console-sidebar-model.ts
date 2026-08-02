export const CONSOLE_SIDEBAR_COLLAPSE_QUERY = "(max-width: 900px)";

export interface ConsoleSidebarPresentation {
  collapsed: boolean;
  allowToggle: boolean;
}

export function resolveConsoleSidebarPresentation(
  manuallyCollapsed: boolean,
  automaticallyCollapsed: boolean,
): ConsoleSidebarPresentation {
  return {
    collapsed: manuallyCollapsed || automaticallyCollapsed,
    allowToggle: !automaticallyCollapsed,
  };
}

export function consoleSidebarRowClasses(
  active: boolean,
  disabled: boolean,
): string {
  if (disabled) {
    return "cursor-default text-muted-foreground opacity-45";
  }
  if (active) {
    return "bg-sidebar-primary text-sidebar-primary-foreground shadow-sm";
  }
  return "text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground";
}
