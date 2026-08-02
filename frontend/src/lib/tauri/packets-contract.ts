import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const PACKETS_CONTRACT_VERSION = 1;
export const PACKETS_MAX_DISPLAY = 500;

export type PacketsCommandError = TechnicalCommandError;
export type PacketsCapturePhase =
  "idle" | "starting" | "running" | "stopping" | "stopped" | "failed";

export interface PacketSnapshot {
  sequence: string;
  timestamp: number;
  source: string;
  destination: string;
  direction: string;
  payloadLen: number;
  declaredIds: number[];
  parsedHits: number;
  note: string;
  decodedText: string;
}

export interface PacketsSnapshot {
  contractVersion: number;
  generation: string;
  sessionGeneration: string;
  packetGeneration: string;
  capturePhase: PacketsCapturePhase;
  eventCount: number;
  observedPacketCount: string;
  packetsWithHits: string;
  retainedPacketCount: number;
  queuedEventCount: number;
  displayLimit: number;
  packets: PacketSnapshot[];
}

export interface PacketsEvent {
  mode: "replace" | "append";
  snapshot: PacketsSnapshot;
}

export function parsePacketsSnapshot(value: unknown): PacketsSnapshot {
  const item = object(value, "packets snapshot");
  const contractVersion = integer(
    item.contractVersion,
    "packets.contractVersion",
  );
  if (contractVersion !== PACKETS_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported packets contract version: ${contractVersion}`,
    );
  }
  const displayLimit = positiveInteger(
    item.displayLimit,
    "packets.displayLimit",
  );
  if (displayLimit > PACKETS_MAX_DISPLAY) {
    throw new TechnicalContractError("packets.displayLimit exceeds UI bounds");
  }
  const packets = list(item.packets, "packets.packets");
  if (packets.length > displayLimit) {
    throw new TechnicalContractError("packets.packets exceeds display limit");
  }
  return {
    contractVersion,
    generation: decimal(item.generation, "packets.generation"),
    sessionGeneration: decimal(
      item.sessionGeneration,
      "packets.sessionGeneration",
    ),
    packetGeneration: decimal(
      item.packetGeneration,
      "packets.packetGeneration",
    ),
    capturePhase: enumValue(
      item.capturePhase,
      ["idle", "starting", "running", "stopping", "stopped", "failed"] as const,
      "packets.capturePhase",
    ),
    eventCount: nonNegativeInteger(item.eventCount, "packets.eventCount"),
    observedPacketCount: decimal(
      item.observedPacketCount,
      "packets.observedPacketCount",
    ),
    packetsWithHits: decimal(item.packetsWithHits, "packets.packetsWithHits"),
    retainedPacketCount: nonNegativeInteger(
      item.retainedPacketCount,
      "packets.retainedPacketCount",
    ),
    queuedEventCount: nonNegativeInteger(
      item.queuedEventCount,
      "packets.queuedEventCount",
    ),
    displayLimit,
    packets: packets.map(parsePacket),
  };
}

export function parsePacketsEvent(value: unknown): PacketsEvent {
  const item = object(value, "packets event");
  if (item.event !== "snapshot" && item.event !== "append") {
    throw new TechnicalContractError("Unsupported packets event");
  }
  return {
    mode: item.event === "snapshot" ? "replace" : "append",
    snapshot: parsePacketsSnapshot(item.payload),
  };
}

export function packetsError(error: unknown): PacketsCommandError {
  return parseTechnicalCommandError(error);
}

function parsePacket(value: unknown, index: number): PacketSnapshot {
  const item = object(value, `packets.packets[${index}]`);
  const declaredIds = list(
    item.declaredIds,
    `packets.packets[${index}].declaredIds`,
  );
  if (declaredIds.length > 256) {
    throw new TechnicalContractError(
      `packets.packets[${index}].declaredIds exceeds bounds`,
    );
  }
  return {
    sequence: decimal(item.sequence, `packets.packets[${index}].sequence`),
    timestamp: nonNegativeNumber(
      item.timestamp,
      `packets.packets[${index}].timestamp`,
    ),
    source: boundedText(item.source, `packets.packets[${index}].source`, 512),
    destination: boundedText(
      item.destination,
      `packets.packets[${index}].destination`,
      512,
    ),
    direction: boundedText(
      item.direction,
      `packets.packets[${index}].direction`,
      64,
    ),
    payloadLen: nonNegativeInteger(
      item.payloadLen,
      `packets.packets[${index}].payloadLen`,
    ),
    declaredIds: declaredIds.map((id, idIndex) =>
      unsigned32(id, `packets.packets[${index}].declaredIds[${idIndex}]`),
    ),
    parsedHits: nonNegativeInteger(
      item.parsedHits,
      `packets.packets[${index}].parsedHits`,
    ),
    note: boundedText(item.note, `packets.packets[${index}].note`, 16_384),
    decodedText: boundedText(
      item.decodedText,
      `packets.packets[${index}].decodedText`,
      2_000_000,
    ),
  };
}

function object(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TechnicalContractError(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}

function list(value: unknown, field: string): unknown[] {
  if (!Array.isArray(value)) {
    throw new TechnicalContractError(`${field} must be an array`);
  }
  return value;
}

function boundedText(value: unknown, field: string, maxLength: number): string {
  if (typeof value !== "string" || value.length > maxLength) {
    throw new TechnicalContractError(`${field} must be bounded text`);
  }
  return value;
}

function decimal(value: unknown, field: string): string {
  if (typeof value !== "string" || value.length > 32 || !/^\d+$/.test(value)) {
    throw new TechnicalContractError(`${field} must be a decimal string`);
  }
  return value;
}

function integer(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value)) {
    throw new TechnicalContractError(`${field} must be a safe integer`);
  }
  return value;
}

function nonNegativeInteger(value: unknown, field: string): number {
  const parsed = integer(value, field);
  if (parsed < 0) {
    throw new TechnicalContractError(`${field} must be non-negative`);
  }
  return parsed;
}

function positiveInteger(value: unknown, field: string): number {
  const parsed = integer(value, field);
  if (parsed <= 0) {
    throw new TechnicalContractError(`${field} must be positive`);
  }
  return parsed;
}

function unsigned32(value: unknown, field: string): number {
  const parsed = nonNegativeInteger(value, field);
  if (parsed > 0xffff_ffff) {
    throw new TechnicalContractError(`${field} must fit u32`);
  }
  return parsed;
}

function nonNegativeNumber(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    throw new TechnicalContractError(
      `${field} must be finite and non-negative`,
    );
  }
  return value;
}

function enumValue<const T extends readonly string[]>(
  value: unknown,
  allowed: T,
  field: string,
): T[number] {
  if (typeof value !== "string" || !allowed.includes(value)) {
    throw new TechnicalContractError(`${field} is invalid`);
  }
  return value as T[number];
}
