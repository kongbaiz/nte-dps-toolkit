import type { ModStudioRuntimeEntry } from "@/lib/tauri/mod-studio-contract";

export type ModRuntimeConsoleFilter =
  "all" | "log" | "event" | "info" | "warning" | "error";

export interface ModRuntimeConsoleClearState {
  generation: string | null;
  throughSequence: string | null;
}

export function visibleRuntimeEntries(
  entries: ModStudioRuntimeEntry[],
  generation: string | null,
  cleared: ModRuntimeConsoleClearState,
  filter: ModRuntimeConsoleFilter,
): ModStudioRuntimeEntry[] {
  const afterClear =
    cleared.generation === generation
      ? entries.filter(
          (entry) =>
            cleared.throughSequence === null ||
            compareRuntimeSequence(entry.sequence, cleared.throughSequence) > 0,
        )
      : entries;
  return afterClear.filter((entry) => runtimeEntryMatches(entry, filter));
}

export function runtimeEntryText(
  entry: ModStudioRuntimeEntry,
  translate: (key: string, arguments_: string[]) => string,
): string {
  if (entry.kind === "log") {
    return entry.messageKey === null
      ? entry.message
      : translate(entry.messageKey, entry.messageArguments);
  }
  return [
    entry.name,
    ...entry.values.map(
      (value, index) =>
        `v${index}=${value}/0x${BigInt(value)
          .toString(16)
          .toUpperCase()
          .padStart(16, "0")}`,
    ),
  ].join(" ");
}

export function runtimeEntryPlainText(
  entry: ModStudioRuntimeEntry,
  translate: (key: string, arguments_: string[]) => string,
): string {
  const level =
    entry.kind === "event"
      ? "EVENT"
      : entry.level === "warning"
        ? "WARN"
        : entry.level.toUpperCase();
  return `${formatRuntimeTimestamp(entry.timestamp100ns)} [${level}] [${entry.modId}] ${runtimeEntryText(entry, translate)}`;
}

export function formatRuntimeTimestamp(timestamp100ns: string): string {
  const windowsEpoch100ns = 116_444_736_000_000_000n;
  const unixMilliseconds =
    (BigInt(timestamp100ns) - windowsEpoch100ns) / 10_000n;
  const timestamp = new Date(Number(unixMilliseconds));
  if (Number.isNaN(timestamp.getTime())) {
    return "--:--:--.---";
  }
  const milliseconds = timestamp.getMilliseconds().toString().padStart(3, "0");
  return `${timestamp.toLocaleTimeString([], {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  })}.${milliseconds}`;
}

function compareRuntimeSequence(left: string, right: string): number {
  if (left.length !== right.length) {
    return left.length - right.length;
  }
  return left === right ? 0 : left < right ? -1 : 1;
}

function runtimeEntryMatches(
  entry: ModStudioRuntimeEntry,
  filter: ModRuntimeConsoleFilter,
): boolean {
  if (filter === "all") {
    return true;
  }
  if (filter === "event" || filter === "log") {
    return entry.kind === filter;
  }
  return entry.kind === "log" && entry.level === filter;
}
