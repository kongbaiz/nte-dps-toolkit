export { CONSOLE_WINDOW_LABEL } from "@/lib/tauri/window-labels";
export const MOD_STUDIO_CONTRACT_VERSION = 5;
export const MOD_STUDIO_SDK_SCHEMA_VERSION = 1;
export const MOD_STUDIO_MAX_DOCUMENTS = 256;
export const MOD_STUDIO_MAX_SOURCE_BYTES = 16_384;
export const MOD_STUDIO_MAX_SDK_SYMBOLS = 128;
export const MOD_STUDIO_MAX_RUNTIME_BATCH_ENTRIES = 36;
export const MOD_STUDIO_MAX_RUNTIME_ENTRIES = 256;
const MOD_ID_PATTERN = /^[a-z0-9._-]{1,31}$/;
const U64_MAX_DECIMAL = "18446744073709551615";

export interface ModStudioDocumentSummary {
  id: string;
  enabled: boolean;
  sourceBytes: number;
  lineCount: number;
}

export interface ModStudioWorkspaceSnapshot {
  contractVersion: number;
  generation: string;
  workspaceLabel: string;
  documents: ModStudioDocumentSummary[];
}

export interface ModStudioDocumentSnapshot {
  contractVersion: number;
  id: string;
  enabled: boolean;
  source: string;
}

export type ModStudioSdkSymbolKind =
  "declaration" | "snippet" | "function" | "property";

export interface ModStudioSdkSymbol {
  label: string;
  insertText: string;
  kind: ModStudioSdkSymbolKind;
  returnType: string | null;
  documentationKey: string;
}

export interface ModStudioSdkSchemaSnapshot {
  contractVersion: number;
  schemaVersion: number;
  symbols: ModStudioSdkSymbol[];
}

export interface ModStudioCommandError {
  code: string;
  messageKey: string;
  messageArguments: string[];
  diagnosticLine: number | null;
}

export type ModStudioRuntimeLevel = "info" | "warning" | "error";

interface ModStudioRuntimeEntryBase {
  sequence: string;
  nativeSequence: string;
  timestamp100ns: string;
  modId: string;
}

export interface ModStudioRuntimeLogEntry extends ModStudioRuntimeEntryBase {
  kind: "log";
  level: ModStudioRuntimeLevel;
  message: string;
  messageKey: string | null;
  messageArguments: string[];
}

export interface ModStudioRuntimeEventEntry extends ModStudioRuntimeEntryBase {
  kind: "event";
  name: string;
  values: string[];
}

export type ModStudioRuntimeEntry =
  ModStudioRuntimeLogEntry | ModStudioRuntimeEventEntry;

export interface ModStudioRuntimeConnectionEvent {
  event: "connection";
  payload: {
    contractVersion: number;
    generation: string;
    connected: boolean;
  };
}

export interface ModStudioRuntimeBatchEvent {
  event: "batch";
  payload: {
    contractVersion: number;
    generation: string;
    entries: ModStudioRuntimeEntry[];
  };
}

export type ModStudioRuntimeEvent =
  ModStudioRuntimeConnectionEvent | ModStudioRuntimeBatchEvent;

export interface ModStudioSubscriptionReceipt {
  subscriptionId: string;
  streamIntervalMs: number;
}

export function parseModStudioWorkspace(
  value: unknown,
): ModStudioWorkspaceSnapshot {
  const workspace = record(value, "Mod workspace");
  const contractVersion = contractVersionOf(workspace);
  const documents = array(workspace.documents, "documents");
  if (documents.length > MOD_STUDIO_MAX_DOCUMENTS) {
    throw new ModStudioContractError(
      `documents exceeds ${MOD_STUDIO_MAX_DOCUMENTS} entries`,
    );
  }

  const parsedDocuments = documents.map((document, index) =>
    parseDocumentSummary(document, `documents[${index}]`),
  );
  const uniqueIds = new Set(parsedDocuments.map((document) => document.id));
  if (uniqueIds.size !== parsedDocuments.length) {
    throw new ModStudioContractError("documents contains duplicate Mod IDs");
  }

  return {
    contractVersion,
    generation: u64DecimalString(workspace.generation, "generation", false),
    workspaceLabel: string(workspace.workspaceLabel, "workspaceLabel"),
    documents: parsedDocuments,
  };
}

export function parseModStudioDocument(
  value: unknown,
): ModStudioDocumentSnapshot {
  const document = record(value, "Mod document");
  const source = string(document.source, "source");
  if (new TextEncoder().encode(source).length > MOD_STUDIO_MAX_SOURCE_BYTES) {
    throw new ModStudioContractError(
      `source exceeds ${MOD_STUDIO_MAX_SOURCE_BYTES} bytes`,
    );
  }
  return {
    contractVersion: contractVersionOf(document),
    id: modId(document.id, "id"),
    enabled: boolean(document.enabled, "enabled"),
    source,
  };
}

export function parseModStudioSdkSchema(
  value: unknown,
): ModStudioSdkSchemaSnapshot {
  const schema = record(value, "Mod SDK schema");
  const contractVersion = contractVersionOf(schema);
  const schemaVersion = positiveInteger(schema.schemaVersion, "schemaVersion");
  if (schemaVersion !== MOD_STUDIO_SDK_SCHEMA_VERSION) {
    throw new ModStudioContractError(
      `Unsupported Mod SDK schema version: ${schemaVersion}`,
    );
  }
  const symbols = array(schema.symbols, "symbols");
  if (symbols.length === 0 || symbols.length > MOD_STUDIO_MAX_SDK_SYMBOLS) {
    throw new ModStudioContractError(
      `symbols must contain 1-${MOD_STUDIO_MAX_SDK_SYMBOLS} entries`,
    );
  }
  const parsedSymbols = symbols.map((symbol, index) =>
    parseSdkSymbol(symbol, `symbols[${index}]`),
  );
  const labels = new Set(parsedSymbols.map((symbol) => symbol.label));
  if (labels.size !== parsedSymbols.length) {
    throw new ModStudioContractError("symbols contains duplicate labels");
  }
  return { contractVersion, schemaVersion, symbols: parsedSymbols };
}

export function parseModStudioCommandError(
  value: unknown,
): ModStudioCommandError {
  const fallback: ModStudioCommandError = {
    code: "unexpected_mod_studio_error",
    messageKey: "The Mod workspace task stopped unexpectedly.",
    messageArguments: [],
    diagnosticLine: null,
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
    diagnosticLine:
      value.diagnosticLine === undefined || value.diagnosticLine === null
        ? null
        : positiveInteger(value.diagnosticLine, "diagnosticLine"),
  };
}

export function parseModStudioRuntimeEvent(
  value: unknown,
): ModStudioRuntimeEvent {
  const event = record(value, "Mod runtime event");
  const payload = record(event.payload, "payload");
  const contractVersion = contractVersionOf(payload);
  const generation = u64DecimalString(
    payload.generation,
    "payload.generation",
    true,
  );
  if (event.event === "connection") {
    return {
      event: "connection",
      payload: {
        contractVersion,
        generation,
        connected: boolean(payload.connected, "payload.connected"),
      },
    };
  }
  if (event.event !== "batch") {
    throw new ModStudioContractError("Mod runtime event type is invalid");
  }
  const entries = array(payload.entries, "payload.entries");
  if (entries.length > MOD_STUDIO_MAX_RUNTIME_BATCH_ENTRIES) {
    throw new ModStudioContractError(
      `payload.entries exceeds ${MOD_STUDIO_MAX_RUNTIME_BATCH_ENTRIES} entries`,
    );
  }
  const parsedEntries = entries.map((entry, index) =>
    parseRuntimeEntry(entry, `payload.entries[${index}]`),
  );
  for (let index = 1; index < parsedEntries.length; index += 1) {
    if (
      compareModStudioSequence(
        parsedEntries[index - 1].sequence,
        parsedEntries[index].sequence,
      ) >= 0
    ) {
      throw new ModStudioContractError(
        "payload.entries must use strictly increasing sequences",
      );
    }
  }
  return {
    event: "batch",
    payload: { contractVersion, generation, entries: parsedEntries },
  };
}

export function parseModStudioSubscriptionReceipt(
  value: unknown,
): ModStudioSubscriptionReceipt {
  const receipt = record(value, "Mod runtime subscription receipt");
  return {
    subscriptionId: string(receipt.subscriptionId, "subscriptionId"),
    streamIntervalMs: positiveInteger(
      receipt.streamIntervalMs,
      "streamIntervalMs",
    ),
  };
}

export function compareModStudioSequence(left: string, right: string): number {
  if (left.length !== right.length) {
    return left.length - right.length;
  }
  return left === right ? 0 : left < right ? -1 : 1;
}

export class ModStudioContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ModStudioContractError";
  }
}

function parseSdkSymbol(value: unknown, field: string): ModStudioSdkSymbol {
  const symbol = record(value, field);
  const kind = string(symbol.kind, `${field}.kind`);
  if (
    kind !== "declaration" &&
    kind !== "snippet" &&
    kind !== "function" &&
    kind !== "property"
  ) {
    throw new ModStudioContractError(`${field}.kind is invalid`);
  }
  const returnType =
    symbol.returnType === null
      ? null
      : boundedString(symbol.returnType, `${field}.returnType`, 64);
  if ((kind === "function" || kind === "property") !== (returnType !== null)) {
    throw new ModStudioContractError(
      `${field}.returnType does not match its symbol kind`,
    );
  }
  return {
    label: boundedString(symbol.label, `${field}.label`, 256),
    insertText: boundedString(symbol.insertText, `${field}.insertText`, 512),
    kind,
    returnType,
    documentationKey: boundedString(
      symbol.documentationKey,
      `${field}.documentationKey`,
      256,
    ),
  };
}

function parseDocumentSummary(
  value: unknown,
  field: string,
): ModStudioDocumentSummary {
  const document = record(value, field);
  return {
    id: modId(document.id, `${field}.id`),
    enabled: boolean(document.enabled, `${field}.enabled`),
    sourceBytes: nonNegativeInteger(
      document.sourceBytes,
      `${field}.sourceBytes`,
    ),
    lineCount: nonNegativeInteger(document.lineCount, `${field}.lineCount`),
  };
}

function parseRuntimeEntry(
  value: unknown,
  field: string,
): ModStudioRuntimeEntry {
  const entry = record(value, field);
  const base = {
    sequence: u64DecimalString(entry.sequence, `${field}.sequence`, true),
    nativeSequence: u64DecimalString(
      entry.nativeSequence,
      `${field}.nativeSequence`,
      true,
    ),
    timestamp100ns: u64DecimalString(
      entry.timestamp100ns,
      `${field}.timestamp100ns`,
      false,
    ),
    modId: modId(entry.modId, `${field}.modId`),
  };
  if (entry.kind === "event") {
    const values = array(entry.values, `${field}.values`);
    if (values.length > 3) {
      throw new ModStudioContractError(`${field}.values exceeds 3 entries`);
    }
    return {
      ...base,
      kind: "event",
      name: modEventName(entry.name, `${field}.name`),
      values: values.map((value, index) =>
        u64DecimalString(value, `${field}.values[${index}]`, false),
      ),
    };
  }
  if (entry.kind !== "log") {
    throw new ModStudioContractError(`${field}.kind is invalid`);
  }
  const messageArguments = array(
    entry.messageArguments,
    `${field}.messageArguments`,
  ).map((argument, index) =>
    string(argument, `${field}.messageArguments[${index}]`),
  );
  const level = string(entry.level, `${field}.level`);
  if (level !== "info" && level !== "warning" && level !== "error") {
    throw new ModStudioContractError(`${field}.level is invalid`);
  }
  const messageKey =
    entry.messageKey === null
      ? null
      : string(entry.messageKey, `${field}.messageKey`);
  return {
    ...base,
    kind: "log",
    level,
    message: boundedString(entry.message, `${field}.message`, 512),
    messageKey,
    messageArguments,
  };
}

function contractVersionOf(value: Record<string, unknown>): number {
  const contractVersion = nonNegativeInteger(
    value.contractVersion,
    "contractVersion",
  );
  if (contractVersion !== MOD_STUDIO_CONTRACT_VERSION) {
    throw new ModStudioContractError(
      `Unsupported Mod Studio contract version: ${contractVersion}`,
    );
  }
  return contractVersion;
}

function modId(value: unknown, field: string): string {
  const parsed = string(value, field);
  if (!MOD_ID_PATTERN.test(parsed)) {
    throw new ModStudioContractError(`${field} must be a valid Mod ID`);
  }
  return parsed;
}

function modEventName(value: unknown, field: string): string {
  const parsed = string(value, field);
  if (!/^[a-z0-9._-]{1,31}$/.test(parsed)) {
    throw new ModStudioContractError(`${field} must be a valid event name`);
  }
  return parsed;
}

function record(value: unknown, field: string): Record<string, unknown> {
  if (!isRecord(value)) {
    throw new ModStudioContractError(`${field} must be an object`);
  }
  return value;
}

function array(value: unknown, field: string): unknown[] {
  if (!Array.isArray(value)) {
    throw new ModStudioContractError(`${field} must be an array`);
  }
  return value;
}

function string(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new ModStudioContractError(`${field} must be a string`);
  }
  return value;
}

function boundedString(
  value: unknown,
  field: string,
  maxLength: number,
): string {
  const parsed = string(value, field);
  if (parsed.length === 0 || parsed.length > maxLength) {
    throw new ModStudioContractError(
      `${field} must contain 1-${maxLength} characters`,
    );
  }
  return parsed;
}

function boolean(value: unknown, field: string): boolean {
  if (typeof value !== "boolean") {
    throw new ModStudioContractError(`${field} must be a boolean`);
  }
  return value;
}

function nonNegativeInteger(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new ModStudioContractError(
      `${field} must be a non-negative safe integer`,
    );
  }
  return value;
}

function positiveInteger(value: unknown, field: string): number {
  const parsed = nonNegativeInteger(value, field);
  if (parsed === 0) {
    throw new ModStudioContractError(`${field} must be positive`);
  }
  return parsed;
}

function u64DecimalString(
  value: unknown,
  field: string,
  positive: boolean,
): string {
  const parsed = string(value, field);
  if (!/^(0|[1-9]\d*)$/.test(parsed)) {
    throw new ModStudioContractError(`${field} must be a decimal string`);
  }
  if (
    parsed.length > U64_MAX_DECIMAL.length ||
    (parsed.length === U64_MAX_DECIMAL.length && parsed > U64_MAX_DECIMAL)
  ) {
    throw new ModStudioContractError(`${field} exceeds u64`);
  }
  if (positive && parsed === "0") {
    throw new ModStudioContractError(`${field} must be positive`);
  }
  return parsed;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
