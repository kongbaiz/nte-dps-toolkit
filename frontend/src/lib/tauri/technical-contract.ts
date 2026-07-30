export const HUD_WINDOW_LABEL = "hud-spike";
export const TECHNICAL_CONTRACT_VERSION = 1;

export interface HudWindowSnapshot {
  passthrough: boolean;
  alwaysOnTop: boolean;
}

export interface TechnicalSnapshot {
  contractVersion: number;
  sequence: string;
  bridgeStatus: string;
  adapterVersion: string;
  windowLabel: string;
  uptimeMs: string;
  streamIntervalMs: number;
  supportedLocales: string[];
  window: HudWindowSnapshot;
}

export interface SubscriptionReceipt {
  subscriptionId: string;
  streamIntervalMs: number;
}

export interface TechnicalCommandError {
  code: string;
  messageKey: string;
  messageArguments: string[];
}

export interface TechnicalSnapshotEvent {
  event: "snapshot";
  payload: TechnicalSnapshot;
}

export type TechnicalEvent = TechnicalSnapshotEvent;

export function parseTechnicalSnapshot(value: unknown): TechnicalSnapshot {
  const snapshot = record(value, "technical snapshot");
  const window = record(snapshot.window, "HUD window snapshot");
  const supportedLocales = array(
    snapshot.supportedLocales,
    "supported locales",
  );
  const contractVersion = integer(snapshot.contractVersion, "contractVersion");

  if (contractVersion !== TECHNICAL_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported technical contract version: ${contractVersion}`,
    );
  }

  return {
    contractVersion,
    sequence: decimalString(snapshot.sequence, "sequence"),
    bridgeStatus: string(snapshot.bridgeStatus, "bridgeStatus"),
    adapterVersion: string(snapshot.adapterVersion, "adapterVersion"),
    windowLabel: string(snapshot.windowLabel, "windowLabel"),
    uptimeMs: decimalString(snapshot.uptimeMs, "uptimeMs"),
    streamIntervalMs: integer(snapshot.streamIntervalMs, "streamIntervalMs"),
    supportedLocales: supportedLocales.map((locale, index) =>
      string(locale, `supportedLocales[${index}]`),
    ),
    window: {
      passthrough: boolean(window.passthrough, "window.passthrough"),
      alwaysOnTop: boolean(window.alwaysOnTop, "window.alwaysOnTop"),
    },
  };
}

export function parseTechnicalEvent(value: unknown): TechnicalEvent {
  const event = record(value, "technical event");

  if (event.event !== "snapshot") {
    throw new TechnicalContractError("Unknown technical event");
  }

  return {
    event: "snapshot",
    payload: parseTechnicalSnapshot(event.payload),
  };
}

export function parseSubscriptionReceipt(value: unknown): SubscriptionReceipt {
  const receipt = record(value, "subscription receipt");

  return {
    subscriptionId: string(receipt.subscriptionId, "subscriptionId"),
    streamIntervalMs: integer(receipt.streamIntervalMs, "streamIntervalMs"),
  };
}

export function parseTechnicalCommandError(
  value: unknown,
): TechnicalCommandError {
  const fallback: TechnicalCommandError = {
    code: "unexpected_technical_error",
    messageKey: "Rust bridge unavailable",
    messageArguments: [],
  };

  if (!isRecord(value)) {
    return fallback;
  }

  const messageArguments = value.messageArguments;
  if (
    typeof value.code !== "string" ||
    typeof value.messageKey !== "string" ||
    !Array.isArray(messageArguments) ||
    !messageArguments.every((argument) => typeof argument === "string")
  ) {
    return fallback;
  }

  return {
    code: value.code,
    messageKey: value.messageKey,
    messageArguments,
  };
}

export class TechnicalContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TechnicalContractError";
  }
}

function record(value: unknown, field: string): Record<string, unknown> {
  if (!isRecord(value)) {
    throw new TechnicalContractError(`${field} must be an object`);
  }
  return value;
}

function array(value: unknown, field: string): unknown[] {
  if (!Array.isArray(value)) {
    throw new TechnicalContractError(`${field} must be an array`);
  }
  return value;
}

function string(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new TechnicalContractError(`${field} must be a string`);
  }
  return value;
}

function decimalString(value: unknown, field: string): string {
  const parsed = string(value, field);
  if (!/^\d+$/.test(parsed)) {
    throw new TechnicalContractError(`${field} must be a decimal string`);
  }
  return parsed;
}

function integer(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value)) {
    throw new TechnicalContractError(`${field} must be a safe integer`);
  }
  return value;
}

function boolean(value: unknown, field: string): boolean {
  if (typeof value !== "boolean") {
    throw new TechnicalContractError(`${field} must be a boolean`);
  }
  return value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
