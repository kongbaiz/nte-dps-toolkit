import { createContractPrimitives } from "@/lib/tauri/contract-primitives";

export const STREAM_PROTOCOL_VERSION = 1;
export const MAX_EVENTS_PER_STREAM_DELIVERY = 2;
export const MAX_STREAM_DELIVERY_BYTES = 14 * 1024 * 1024;

const { nonEmptyString, positiveU64DecimalString, positiveInteger, record, string } =
  createContractPrimitives((message) => {
    throw new StreamContractError(message);
  });

export const STREAM_KINDS = [
  "technical",
  "diagnostics",
  "history",
  "mainDps",
  "mainDpsDetail",
  "emptyCurtain",
  "modStudioRuntime",
  "packets",
  "settings",
  "skills",
  "timeline",
] as const;

export type StreamKind = (typeof STREAM_KINDS)[number];

export interface StreamSubscriptionReceipt {
  subscriptionId: string;
  streamKind: StreamKind;
  streamIntervalMs: number;
  streamProtocolVersion: number;
  streamGeneration: string;
}

export class StreamContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "StreamContractError";
  }
}

export function parseStreamSubscriptionReceipt(
  value: unknown,
): StreamSubscriptionReceipt {
  const receipt = record(value, "stream subscription receipt");
  return {
    subscriptionId: subscriptionId(receipt.subscriptionId),
    streamKind: parseStreamKind(receipt.streamKind),
    streamIntervalMs: positiveInteger(
      receipt.streamIntervalMs,
      "streamIntervalMs",
    ),
    streamProtocolVersion: protocolVersion(receipt.streamProtocolVersion),
    streamGeneration: positiveU64DecimalString(
      receipt.streamGeneration,
      "streamGeneration",
    ),
  };
}

export function parseStreamDelivery(value: unknown): unknown[] {
  const delivery = record(value, "stream delivery");
  protocolVersion(delivery.streamProtocolVersion);
  if (!Array.isArray(delivery.events)) {
    throw new StreamContractError("stream delivery events must be an array");
  }
  if (
    delivery.events.length === 0 ||
    delivery.events.length > MAX_EVENTS_PER_STREAM_DELIVERY
  ) {
    throw new StreamContractError(
      `stream delivery must contain 1..${MAX_EVENTS_PER_STREAM_DELIVERY} events`,
    );
  }
  return delivery.events;
}

function protocolVersion(value: unknown): number {
  const version = positiveInteger(value, "streamProtocolVersion");
  if (version !== STREAM_PROTOCOL_VERSION) {
    throw new StreamContractError(
      `unsupported stream protocol version: ${version}`,
    );
  }
  return version;
}

function subscriptionId(value: unknown): string {
  const parsed = nonEmptyString(value, "subscriptionId");
  if (!/^[A-Za-z0-9_-]{1,64}$/.test(parsed)) {
    throw new StreamContractError("subscriptionId is invalid");
  }
  return parsed;
}

function parseStreamKind(value: unknown): StreamKind {
  const parsed = string(value, "streamKind");
  if (!STREAM_KINDS.includes(parsed as StreamKind)) {
    throw new StreamContractError("streamKind is not supported");
  }
  return parsed as StreamKind;
}
