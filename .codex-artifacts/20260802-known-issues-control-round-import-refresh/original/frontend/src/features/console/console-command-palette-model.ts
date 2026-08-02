import type {
  AccentId,
  DensityId,
  LayoutProfileId,
  ThemePresetId,
} from "@/lib/tauri/settings-contract";

import { CONSOLE_PAGE_IDS, type ConsolePageId } from "./console-navigation";

export type ConsoleCommandAction =
  | { kind: "navigate"; page: ConsolePageId }
  | { kind: "open-abyss" }
  | { kind: "open-hud-editor" }
  | { kind: "import"; format: "json" | "pcapng" }
  | { kind: "export"; format: "json" | "pcapng" | "team" }
  | { kind: "save-history" }
  | { kind: "layout"; profile: LayoutProfileId }
  | { kind: "theme-preset"; value: ThemePresetId }
  | { kind: "accent"; value: AccentId }
  | { kind: "density"; value: DensityId }
  | { kind: "toggle-reduced-motion" }
  | { kind: "unavailable" };

export interface ConsoleCommandDefinition {
  id: string;
  titleKey: string;
  categoryKey: string;
  keywords: readonly string[];
  action: ConsoleCommandAction;
}

export interface ConsoleCommandRowClasses {
  row: string;
  secondary: string;
}

export function consoleCommandRowClasses(
  disabled: boolean,
  selected: boolean,
): ConsoleCommandRowClasses {
  if (disabled) {
    return {
      row: "cursor-default opacity-45",
      secondary: "text-muted-foreground",
    };
  }
  if (selected) {
    return {
      row: "bg-accent text-accent-foreground",
      secondary: "text-accent-foreground/75",
    };
  }
  return {
    row: "hover:bg-muted",
    secondary: "text-muted-foreground",
  };
}

const pageMetadata: Record<ConsolePageId, [string, string]> = {
  settings: ["console.settings", "Open Settings"],
  history: ["console.history", "Open History"],
  timeline: ["console.timeline", "Open Timeline"],
  skills: ["console.skills", "Open Skills"],
  "empty-curtain": ["console.loadout", "Open Console Loadout"],
  "character-data": ["console.characters", "Open Character Data"],
  "encrypted-ini": ["console.ini", "Open Encrypted INI"],
  packets: ["console.packets", "Open Packets"],
  resources: ["console.resources", "Open Resources"],
  diagnostics: ["console.diagnostics", "Open Diagnostics"],
  "mod-studio": ["console.mods", "Open Mod Studio"],
};

const unavailable = (
  id: string,
  titleKey: string,
  categoryKey: string,
  keywords: readonly string[],
): ConsoleCommandDefinition => ({
  id,
  titleKey,
  categoryKey,
  keywords,
  action: { kind: "unavailable" },
});

export const CONSOLE_COMMANDS: readonly ConsoleCommandDefinition[] = [
  unavailable("capture.toggle", "Start / Stop Capture", "Combat", [
    "capture",
    "start",
    "stop",
  ]),
  unavailable("session.reset", "Reset Session", "Combat", ["clear", "undo"]),
  unavailable("hud.toggle", "Toggle Combat HUD", "Windows", [
    "overlay",
    "game",
  ]),
  unavailable("passthrough.toggle", "Toggle Mouse Passthrough", "Windows", [
    "mouse",
    "click",
  ]),
  unavailable("capture.pause", "Pause / Resume Processing", "Combat", [
    "pause",
    "resume",
  ]),
  unavailable("window.pin", "Toggle Always on Top", "Windows", ["pin", "top"]),
  unavailable("theme.toggle", "Toggle Light / Dark Theme", "Appearance", [
    "theme",
    "light",
    "dark",
  ]),
  {
    id: "motion.toggle",
    titleKey: "Toggle Reduced Motion",
    categoryKey: "Appearance",
    keywords: ["animation", "accessibility"],
    action: { kind: "toggle-reduced-motion" },
  },
  unavailable("action.undo", "Undo Last Action", "General", [
    "restore",
    "reset",
    "delete",
  ]),
  {
    id: "window.abyss",
    titleKey: "Open Abyss Overview",
    categoryKey: "Windows",
    keywords: ["abyss", "prediction"],
    action: { kind: "open-abyss" },
  },
  {
    id: "window.hud_editor",
    titleKey: "Open HUD Editor",
    categoryKey: "Windows",
    keywords: ["hud", "overlay", "edit"],
    action: { kind: "open-hud-editor" },
  },
  unavailable("window.team_details", "Open Team Combat Details", "Windows", [
    "hits",
    "detail",
  ]),
  {
    id: "import.pcapng",
    titleKey: "Import PCAPNG",
    categoryKey: "Data",
    keywords: ["replay", "wireshark"],
    action: { kind: "import", format: "pcapng" },
  },
  {
    id: "import.capture_json",
    titleKey: "Import Capture JSON",
    categoryKey: "Data",
    keywords: ["replay", "json"],
    action: { kind: "import", format: "json" },
  },
  {
    id: "history.save",
    titleKey: "Save History Summary",
    categoryKey: "Data",
    keywords: ["history", "snapshot"],
    action: { kind: "save-history" },
  },
  {
    id: "team.export",
    titleKey: "Export Team DPS Data",
    categoryKey: "Data",
    keywords: ["json", "abyss"],
    action: { kind: "export", format: "team" },
  },
  {
    id: "capture.export_parsed",
    titleKey: "Export Parsed JSON",
    categoryKey: "Data",
    keywords: ["capture", "json", "export"],
    action: { kind: "export", format: "json" },
  },
  {
    id: "capture.export_raw",
    titleKey: "Save Full PCAPNG As",
    categoryKey: "Data",
    keywords: ["capture", "pcapng", "raw"],
    action: { kind: "export", format: "pcapng" },
  },
  unavailable("capture.open_logs", "Open Capture Logs Folder", "Data", [
    "capture",
    "logs",
    "folder",
  ]),
  ...CONSOLE_PAGE_IDS.map((page): ConsoleCommandDefinition => ({
    id: pageMetadata[page][0],
    titleKey: pageMetadata[page][1],
    categoryKey: "Console",
    keywords: ["console", "tab", "page"],
    action: { kind: "navigate", page },
  })),
  ...(["combat", "review", "research"] as const).map(
    (profile): ConsoleCommandDefinition => ({
      id: `layout.${profile}`,
      titleKey: `Apply ${capitalized(profile)} Layout`,
      categoryKey: "Windows",
      keywords: ["profile", profile],
      action: { kind: "layout", profile },
    }),
  ),
  ...(["zinc", "tactical", "high-contrast"] as const).map(
    (value): ConsoleCommandDefinition => ({
      id: `theme.preset.${value.replace("-", "_")}`,
      titleKey:
        value === "high-contrast"
          ? "Use High Contrast Theme"
          : `Use ${capitalized(value)} Theme`,
      categoryKey: "Appearance",
      keywords: ["theme", value],
      action: { kind: "theme-preset", value },
    }),
  ),
  ...(["zinc", "blue", "violet", "orange", "green"] as const).map(
    (value): ConsoleCommandDefinition => ({
      id: `accent.${value}`,
      titleKey: `Use ${capitalized(value)} Accent`,
      categoryKey: "Appearance",
      keywords: ["color", value],
      action: { kind: "accent", value },
    }),
  ),
  ...(["compact", "cozy", "comfortable"] as const).map(
    (value): ConsoleCommandDefinition => ({
      id: `density.${value}`,
      titleKey: `Use ${capitalized(value)} Density`,
      categoryKey: "Appearance",
      keywords: ["spacing", value],
      action: { kind: "density", value },
    }),
  ),
];

export function filterConsoleCommands(
  commands: readonly ConsoleCommandDefinition[],
  query: string,
  translate: (key: string) => string,
): ConsoleCommandDefinition[] {
  const normalized = query.trim().toLocaleLowerCase();
  if (normalized.length === 0) return [...commands];
  return commands.filter((command) =>
    [
      command.titleKey,
      translate(command.titleKey),
      command.categoryKey,
      translate(command.categoryKey),
      ...command.keywords,
    ].some((candidate) => fuzzyTextMatches(candidate, normalized)),
  );
}

export function nextEnabledCommandIndex(
  commands: readonly ConsoleCommandDefinition[],
  current: number,
  offset: -1 | 1,
): number {
  if (commands.length === 0) return 0;
  for (let step = 1; step <= commands.length; step += 1) {
    const index = (current + step * offset + commands.length) % commands.length;
    if (commands[index].action.kind !== "unavailable") return index;
  }
  return 0;
}

function fuzzyTextMatches(text: string, query: string): boolean {
  const candidate = text.toLocaleLowerCase();
  if (candidate.includes(query)) return true;
  let index = 0;
  for (const character of candidate) {
    if (character === query[index]) index += 1;
    if (index === query.length) return true;
  }
  return false;
}

function capitalized(value: string): string {
  return value
    .split("-")
    .map((part) => part[0].toUpperCase() + part.slice(1))
    .join(" ");
}
