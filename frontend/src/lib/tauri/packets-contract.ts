import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const PACKETS_CONTRACT_VERSION = 2;
export const PACKETS_MAX_DISPLAY = 500;
export const PACKETS_MAX_SOURCE_BYTES = 512;
export const PACKETS_MAX_DESTINATION_BYTES = 512;
export const PACKETS_MAX_DIRECTION_BYTES = 64;
export const PACKETS_MAX_NOTE_BYTES = 16_384;
export const PACKETS_MAX_DECODED_TEXT_BYTES = 2_000_000;
export const PACKETS_MAX_DECLARED_IDS = 256;

const {
  array: list,
  boundedUtf8String,
  decimalString32: decimal,
  enumValue,
  integer,
  nonNegativeInteger,
  nonNegativeNumber,
  positiveInteger,
  record: object,
  unsigned32,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

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
  omittedTextBytes: string;
  omittedDeclaredIdCount: string;
}

export interface PacketsSnapshot {
  contractVersion: number;
  generation: string;
  sessionGeneration: string;
  packetGeneration: string;
  firstDisplaySequence: string;
  capturePhase: PacketsCapturePhase;
  eventCount: number;
  observedPacketCount: string;
  packetsWithHits: string;
  retainedPacketCount: number;
  queuedEventCount: number;
  displayLimit: number;
  truncatedPacketCount: number;
  omittedTextBytes: string;
  omittedDeclaredIdCount: string;
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
  const parsedPackets = packets.map(parsePacket);
  const firstDisplaySequence = decimal(
    item.firstDisplaySequence,
    "packets.firstDisplaySequence",
  );
  if (
    parsedPackets.some(
      (packet) => BigInt(packet.sequence) < BigInt(firstDisplaySequence),
    )
  ) {
    throw new TechnicalContractError(
      "packets packet sequence precedes firstDisplaySequence",
    );
  }
  const truncatedPacketCount = nonNegativeInteger(
    item.truncatedPacketCount,
    "packets.truncatedPacketCount",
  );
  if (truncatedPacketCount > parsedPackets.length) {
    throw new TechnicalContractError(
      "packets.truncatedPacketCount exceeds packet rows",
    );
  }
  const observedTruncatedPacketCount = parsedPackets.filter(
    (packet) =>
      packet.omittedTextBytes !== "0" || packet.omittedDeclaredIdCount !== "0",
  ).length;
  if (truncatedPacketCount !== observedTruncatedPacketCount) {
    throw new TechnicalContractError(
      "packets.truncatedPacketCount does not match packet omission metadata",
    );
  }
  const omittedTextBytes = decimal(
    item.omittedTextBytes,
    "packets.omittedTextBytes",
  );
  const omittedDeclaredIdCount = decimal(
    item.omittedDeclaredIdCount,
    "packets.omittedDeclaredIdCount",
  );
  const rowOmittedTextBytes = parsedPackets.reduce(
    (total, packet) => total + BigInt(packet.omittedTextBytes),
    0n,
  );
  const rowOmittedDeclaredIds = parsedPackets.reduce(
    (total, packet) => total + BigInt(packet.omittedDeclaredIdCount),
    0n,
  );
  if (rowOmittedTextBytes !== BigInt(omittedTextBytes)) {
    throw new TechnicalContractError(
      "packets.omittedTextBytes does not match packet rows",
    );
  }
  if (rowOmittedDeclaredIds !== BigInt(omittedDeclaredIdCount)) {
    throw new TechnicalContractError(
      "packets.omittedDeclaredIdCount does not match packet rows",
    );
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
    firstDisplaySequence,
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
    truncatedPacketCount,
    omittedTextBytes,
    omittedDeclaredIdCount,
    packets: parsedPackets,
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
  if (declaredIds.length > PACKETS_MAX_DECLARED_IDS) {
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
    source: boundedUtf8String(
      item.source,
      `packets.packets[${index}].source`,
      PACKETS_MAX_SOURCE_BYTES,
      { allowEmpty: true },
    ),
    destination: boundedUtf8String(
      item.destination,
      `packets.packets[${index}].destination`,
      PACKETS_MAX_DESTINATION_BYTES,
      { allowEmpty: true },
    ),
    direction: boundedUtf8String(
      item.direction,
      `packets.packets[${index}].direction`,
      PACKETS_MAX_DIRECTION_BYTES,
      { allowEmpty: true },
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
    note: boundedUtf8String(
      item.note,
      `packets.packets[${index}].note`,
      PACKETS_MAX_NOTE_BYTES,
      { allowEmpty: true },
    ),
    decodedText: boundedUtf8String(
      item.decodedText,
      `packets.packets[${index}].decodedText`,
      PACKETS_MAX_DECODED_TEXT_BYTES,
      { allowEmpty: true },
    ),
    omittedTextBytes: decimal(
      item.omittedTextBytes,
      `packets.packets[${index}].omittedTextBytes`,
    ),
    omittedDeclaredIdCount: decimal(
      item.omittedDeclaredIdCount,
      `packets.packets[${index}].omittedDeclaredIdCount`,
    ),
  };
}
