import type {
  TechnicalCommandError,
  TechnicalSnapshot,
} from "@/lib/tauri/technical-contract";

export type TechnicalPageState =
  | { status: "loading" }
  | { status: "error"; error: TechnicalCommandError }
  | { status: "ready"; snapshot: TechnicalSnapshot };

export type BridgeTone = "ready" | "degraded" | "unknown";

export function acceptSnapshot(
  current: TechnicalPageState,
  incoming: TechnicalSnapshot,
): TechnicalPageState {
  if (
    current.status === "ready" &&
    BigInt(incoming.sequence) <= BigInt(current.snapshot.sequence)
  ) {
    return current;
  }

  return { status: "ready", snapshot: incoming };
}

export function bridgeTone(status: string): BridgeTone {
  switch (status) {
    case "ready":
      return "ready";
    case "degraded":
      return "degraded";
    default:
      return "unknown";
  }
}

export function localeSummary(locales: readonly string[]): string | undefined {
  return locales.length === 0 ? undefined : locales.join(" · ");
}
