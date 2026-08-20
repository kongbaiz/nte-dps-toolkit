import type {
  PacketSnapshot,
  PacketsEvent,
  PacketsSnapshot,
} from "@/lib/tauri/packets-contract";

export type PacketsContentKind =
  "loading" | "error" | "empty" | "filtered-empty" | "list";

export function packetsContentKind(
  status: "loading" | "error" | "ready",
  packetCount: number,
  filteredPacketCount: number,
): PacketsContentKind {
  if (status === "loading") return "loading";
  if (status === "error") return "error";
  if (packetCount === 0) return "empty";
  return filteredPacketCount === 0 ? "filtered-empty" : "list";
}

export function mergePacketsEvent(
  current: PacketsSnapshot | null,
  event: PacketsEvent,
): PacketsSnapshot {
  const incoming = event.snapshot;
  if (
    current !== null &&
    compareDecimal(incoming.generation, current.generation) <= 0
  ) {
    return current;
  }
  if (
    current === null ||
    event.mode === "replace" ||
    incoming.sessionGeneration !== current.sessionGeneration
  ) {
    return incoming;
  }

  const packets = new Map(
    current.packets.map((packet) => [packet.sequence, packet] as const),
  );
  for (const packet of incoming.packets) packets.set(packet.sequence, packet);
  const merged = [...packets.values()]
    .filter(
      (packet) =>
        compareDecimal(packet.sequence, incoming.firstDisplaySequence) >= 0,
    )
    .sort((left, right) => compareDecimal(left.sequence, right.sequence));
  if (merged.length > incoming.displayLimit) {
    throw new Error("Packets server window exceeds its declared display limit");
  }
  return { ...incoming, packets: merged };
}

export function packetMatches(
  packet: PacketSnapshot,
  normalizedQuery: string,
  hitsOnly: boolean,
): boolean {
  if (hitsOnly && packet.parsedHits === 0) return false;
  if (normalizedQuery.length === 0) return true;
  return packetSearchText(packet).includes(normalizedQuery);
}

export function normalizePacketSearch(query: string): string {
  return query.trim().toLocaleLowerCase();
}

export function formatPacketTimestamp(timestamp: number): string {
  const date = new Date(timestamp * 1_000);
  if (Number.isNaN(date.getTime())) return "--:--:--.---";
  const milliseconds = date.getMilliseconds().toString().padStart(3, "0");
  return `${date.toLocaleTimeString(undefined, { hour12: false })}.${milliseconds}`;
}

export function compareDecimal(left: string, right: string): number {
  const normalizedLeft = left.replace(/^0+(?=\d)/, "");
  const normalizedRight = right.replace(/^0+(?=\d)/, "");
  return normalizedLeft.length === normalizedRight.length
    ? normalizedLeft.localeCompare(normalizedRight)
    : normalizedLeft.length - normalizedRight.length;
}

function packetSearchText(packet: PacketSnapshot): string {
  return [
    packet.source,
    packet.destination,
    packet.direction,
    packet.declaredIds.join(" "),
    packet.note,
    packet.decodedText,
  ]
    .join(" ")
    .toLocaleLowerCase();
}
