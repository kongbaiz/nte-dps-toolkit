import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
export const STREAM_PROTOCOL_VERSION = 1;
export const MAX_IN_FLIGHT_STREAM_DELIVERIES = 1;
export const MAX_STREAM_DELIVERY_BYTES = 16 * 1024 * 1024;
export const MAX_EVENTS_PER_STREAM_DELIVERY = 2;

const {
  nonEmptyString,
  positiveU64DecimalString: positiveDecimalString,
  positiveInteger,
  record,
  string,
} = createContractPrimitives((message) => {
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

export interface StreamReadySignal {
  streamProtocolVersion: number;
  streamKind: StreamKind;
  subscriptionId: string;
  streamGeneration: string;
  deliverySequence: string;
}

export interface StreamSubscriptionReceipt {
  subscriptionId: string;
  streamKind: StreamKind;
  streamIntervalMs: number;
  streamProtocolVersion: number;
  streamGeneration: string;
  maxInFlightDeliveries: number;
  maxDeliveryBytes: number;
}

export interface StreamAckReceipt {
  accepted: boolean;
}

export class StreamContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "StreamContractError";
  }
}

export function parseStreamReadySignal(value: unknown): StreamReadySignal {
  const signal = record(value, "stream ready signal");
  const streamProtocolVersion = protocolVersion(signal.streamProtocolVersion);
  const streamKind = parseStreamKind(signal.streamKind);

  return {
    streamProtocolVersion,
    streamKind,
    subscriptionId: subscriptionId(signal.subscriptionId),
    streamGeneration: positiveDecimalString(
      signal.streamGeneration,
      "streamGeneration",
    ),
    deliverySequence: positiveDecimalString(
      signal.deliverySequence,
      "deliverySequence",
    ),
  };
}

export function parseStreamSubscriptionReceipt(
  value: unknown,
): StreamSubscriptionReceipt {
  const receipt = record(value, "stream subscription receipt");
  const maxInFlightDeliveries = positiveInteger(
    receipt.maxInFlightDeliveries,
    "maxInFlightDeliveries",
  );
  if (maxInFlightDeliveries !== MAX_IN_FLIGHT_STREAM_DELIVERIES) {
    throw new StreamContractError(
      `maxInFlightDeliveries must be ${MAX_IN_FLIGHT_STREAM_DELIVERIES}`,
    );
  }
  const maxDeliveryBytes = positiveInteger(
    receipt.maxDeliveryBytes,
    "maxDeliveryBytes",
  );
  if (maxDeliveryBytes !== MAX_STREAM_DELIVERY_BYTES) {
    throw new StreamContractError(
      `maxDeliveryBytes must be ${MAX_STREAM_DELIVERY_BYTES}`,
    );
  }

  return {
    subscriptionId: subscriptionId(receipt.subscriptionId),
    streamKind: parseStreamKind(receipt.streamKind),
    streamIntervalMs: positiveInteger(
      receipt.streamIntervalMs,
      "streamIntervalMs",
    ),
    streamProtocolVersion: protocolVersion(receipt.streamProtocolVersion),
    streamGeneration: positiveDecimalString(
      receipt.streamGeneration,
      "streamGeneration",
    ),
    maxInFlightDeliveries,
    maxDeliveryBytes,
  };
}

export function parseStreamDelivery(
  value: unknown,
  maxDeliveryBytes: number,
): unknown[] {
  if (!(value instanceof ArrayBuffer)) {
    throw new StreamContractError("stream delivery must be an ArrayBuffer");
  }
  const boundedBytes = positiveInteger(maxDeliveryBytes, "maxDeliveryBytes");
  if (
    boundedBytes > MAX_STREAM_DELIVERY_BYTES ||
    value.byteLength > boundedBytes
  ) {
    throw new StreamContractError("stream delivery exceeds its byte budget");
  }

  let decoded: string;
  try {
    decoded = new TextDecoder("utf-8", { fatal: true }).decode(value);
  } catch {
    throw new StreamContractError("stream delivery is not valid UTF-8");
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(decoded) as unknown;
  } catch {
    throw new StreamContractError("stream delivery is not valid JSON");
  }
  const delivery = record(parsed, "stream delivery");
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

export function parseStreamAckReceipt(value: unknown): StreamAckReceipt {
  const receipt = record(value, "stream ACK receipt");
  if (typeof receipt.accepted !== "boolean") {
    throw new StreamContractError("stream ACK accepted must be a boolean");
  }
  return { accepted: receipt.accepted };
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
