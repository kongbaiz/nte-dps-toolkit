import {
  createContractPrimitives,
  isCanonicalSemver,
} from "@/lib/tauri/contract-primitives";
import {
  parseStreamSubscriptionReceipt,
  type StreamSubscriptionReceipt,
} from "@/lib/tauri/stream-contract";

export const MOD_STUDIO_CONTRACT_VERSION = 13;
export const MOD_MARKET_CONTRACT_VERSION = 11;
export const MOD_STUDIO_DIRECTORY_CONTRACT_VERSION = 1;
export const MOD_STUDIO_LOADING_METHOD_CONTRACT_VERSION = 1;
export const MOD_LOADER_RUNTIME_CONTRACT_VERSION = 1;
export const MOD_STUDIO_SDK_SCHEMA_VERSION = 2;
export const MOD_STUDIO_MAX_DOCUMENTS = 256;
export const MOD_STUDIO_MAX_SOURCE_BYTES = 16_384;
export const MOD_STUDIO_MAX_SDK_SYMBOLS = 128;
export const MOD_STUDIO_MAX_RUNTIME_BATCH_ENTRIES = 36;
export const MOD_STUDIO_MAX_RUNTIME_ENTRIES = 256;
const MOD_ID_PATTERN = /^[a-z0-9._-]{1,31}$/;

export class ModStudioContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ModStudioContractError";
  }
}

const {
  array,
  boolean,
  boundedString,
  exactFields,
  enumValue,
  isRecord,
  nonNegativeInteger,
  nullableEnumValue,
  positiveInteger,
  record,
  string,
  u64DecimalString,
} = createContractPrimitives((message) => {
  throw new ModStudioContractError(message);
});

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

export type ModStudioGameRegion = "china" | "global";

export interface ModStudioGameStatus {
  region: ModStudioGameRegion;
  installed: boolean;
  current: boolean;
}

export interface ModStudioDeploymentSnapshot {
  contractVersion: number;
  installations: number;
  installed: number;
  current: number;
  sourceAvailable: boolean;
  games: ModStudioGameStatus[];
}

export interface ModStudioDirectorySelectionSnapshot {
  selected: boolean;
  path: string | null;
  deployment: ModStudioDeploymentSnapshot;
}

export interface ModStudioGameDirectorySnapshot {
  contractVersion: number;
  region: ModStudioGameRegion;
  path: string | null;
}

export type ModLoadingMethod = "proxy" | "loader";

export interface ModStudioLoadingMethodPreferenceSnapshot {
  contractVersion: number;
  method: ModLoadingMethod;
  riskAcknowledged: boolean;
}

export type ModLoaderRuntimePhase =
  "missingLoader" | "missingPayload" | "stopped" | "running";

export interface ModLoaderRuntimeSnapshot {
  contractVersion: number;
  phase: ModLoaderRuntimePhase;
  loaderPresent: boolean;
  payloadPresent: boolean;
  loaderFileName: "nte-mod-loader.exe";
  payloadRelativePath: "plugins/dwmapi.dll";
  placement: "applicationDirectory";
}

export interface ModMarketItem {
  id: string;
  bindings: string[];
  localizations: {
    en: ModMarketLocalizedText;
    "zh-CN": ModMarketLocalizedText;
    ja: ModMarketLocalizedText;
  };
  version: string;
  author: string;
  capabilities: string[];
  packageSize: number;
  localState: ModMarketLocalState;
}

const MOD_MARKET_UNREADABLE_MESSAGE_KEY_BY_CODE = {
  mod_workspace_invalid: "The Mod workspace data is invalid.",
  mod_workspace_read_failed: "Failed to read the Mod workspace.",
} as const;

export type ModMarketUnreadableCode =
  keyof typeof MOD_MARKET_UNREADABLE_MESSAGE_KEY_BY_CODE;
export type ModMarketUnreadableMessageKey =
  (typeof MOD_MARKET_UNREADABLE_MESSAGE_KEY_BY_CODE)[ModMarketUnreadableCode];

export type ModMarketLocalState =
  | { status: "notInstalled" }
  | { status: "installed"; enabled: boolean; current: boolean }
  | {
      status: "unreadable";
      code: ModMarketUnreadableCode;
      messageKey: ModMarketUnreadableMessageKey;
    };

export interface ModMarketLocalizedText {
  name: string;
  summary: string;
}

export interface ModMarketCatalogSnapshot {
  contractVersion: number;
  publishedAt: string;
  privacyMode: "anonymous-read-only";
  mods: ModMarketItem[];
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
    status: ModStudioRuntimeConnectionStatus;
    bootstrapErrorCode: ModStudioBootstrapErrorCode | null;
    probeErrorCode: ModStudioRuntimeProbeErrorCode | null;
    probeOsErrorCode: number | null;
  };
}

const MOD_STUDIO_RUNTIME_PROBE_ERROR_CODES = [
  "RUNTIME_EVENT_ACCESS_DENIED",
  "RUNTIME_EVENT_OPEN_FAILED",
  "IPC_CLIENT_UNAVAILABLE",
  "IPC_PIPE_ACCESS_DENIED",
  "IPC_PIPE_OPEN_FAILED",
  "POLL_WORKER_FAILED",
  "RUNTIME_SUBSCRIPTION_FAILED",
] as const;

export type ModStudioRuntimeProbeErrorCode =
  (typeof MOD_STUDIO_RUNTIME_PROBE_ERROR_CODES)[number];

export type ModStudioRuntimeConnectionStatus =
  | "connected"
  | "loaderPresent"
  | "waiting"
  | "acknowledgementRequired"
  | "probeFailed"
  | "bootstrapFailed";

const MOD_STUDIO_BOOTSTRAP_ERROR_CODES = [
  "INVALID_REQUEST",
  "FILE_SYSTEM",
  "INVALID_PLUGIN_IMAGE",
  "PROCESS_OPEN_FAILED",
  "PROCESS_IDENTITY_FAILED",
  "PROCESS_SNAPSHOT_FAILED",
  "PROCESS_ENUMERATION_FAILED",
  "PROCESS_AMBIGUOUS",
  "TARGET_MISMATCH",
  "MODULE_SNAPSHOT_FAILED",
  "MODULE_ENUMERATION_FAILED",
  "MODULE_NOT_LOADED",
  "MODULE_PATH_MISMATCH",
  "MODULE_AMBIGUOUS",
  "MODULE_IMAGE_MISMATCH",
  "REMOTE_THREAD_FAILED",
  "REMOTE_THREAD_TIMED_OUT",
  "REMOTE_THREAD_WAIT_FAILED",
  "REMOTE_THREAD_EXIT_FAILED",
  "NOT_GAME_HOST",
  "INITIALIZATION_IN_PROGRESS",
  "INITIALIZATION_FAILED",
  "UNEXPECTED_STATUS",
] as const;

export type ModStudioBootstrapErrorCode =
  (typeof MOD_STUDIO_BOOTSTRAP_ERROR_CODES)[number];

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

export type ModStudioSubscriptionReceipt = StreamSubscriptionReceipt;

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

export function parseModStudioDeployment(
  value: unknown,
): ModStudioDeploymentSnapshot {
  const deployment = record(value, "Mod loader deployment");
  const games = array(deployment.games, "games").map((game, index) => {
    const parsed = record(game, `games[${index}]`);
    return {
      region: gameRegion(parsed.region, `games[${index}].region`),
      installed: boolean(parsed.installed, `games[${index}].installed`),
      current: boolean(parsed.current, `games[${index}].current`),
    };
  });
  if (new Set(games.map((game) => game.region)).size !== games.length) {
    throw new ModStudioContractError("games contains duplicate regions");
  }
  const installations = nonNegativeInteger(
    deployment.installations,
    "installations",
  );
  const installed = nonNegativeInteger(deployment.installed, "installed");
  const current = nonNegativeInteger(deployment.current, "current");
  if (installed > installations || current > installed) {
    throw new ModStudioContractError(
      "Mod loader deployment counts are invalid",
    );
  }
  return {
    contractVersion: contractVersionOf(deployment),
    installations,
    installed,
    current,
    sourceAvailable: boolean(deployment.sourceAvailable, "sourceAvailable"),
    games,
  };
}

export function parseModStudioDirectorySelection(
  value: unknown,
): ModStudioDirectorySelectionSnapshot {
  const selection = record(value, "Game directory selection");
  const selected = boolean(selection.selected, "selected");
  const path =
    selection.path === null
      ? null
      : boundedString(selection.path, "path", 32_768);
  if (selected !== (path !== null)) {
    throw new ModStudioContractError("selected and path do not match");
  }
  return {
    selected,
    path,
    deployment: parseModStudioDeployment(selection.deployment),
  };
}

export function parseModStudioGameDirectory(
  value: unknown,
): ModStudioGameDirectorySnapshot {
  const snapshot = record(value, "Mod Studio game directory");
  const contractVersion = contractVersionOf(
    snapshot,
    MOD_STUDIO_DIRECTORY_CONTRACT_VERSION,
  );
  const region = gameRegion(snapshot.region, "region");
  const path =
    snapshot.path === null
      ? null
      : boundedString(snapshot.path, "path", 32_768);
  return { contractVersion, region, path };
}

export function parseModStudioLoadingMethodPreference(
  value: unknown,
): ModStudioLoadingMethodPreferenceSnapshot {
  const snapshot = record(value, "Mod Studio loading method");
  const contractVersion = contractVersionOf(
    snapshot,
    MOD_STUDIO_LOADING_METHOD_CONTRACT_VERSION,
  );
  const method = string(snapshot.method, "method");
  if (method !== "proxy" && method !== "loader") {
    throw new ModStudioContractError("method is invalid");
  }
  const riskAcknowledged = boolean(
    snapshot.riskAcknowledged,
    "riskAcknowledged",
  );
  return { contractVersion, method, riskAcknowledged };
}

export function parseModLoaderRuntime(
  value: unknown,
): ModLoaderRuntimeSnapshot {
  const snapshot = record(value, "Mod Loader runtime");
  const contractVersion = contractVersionOf(
    snapshot,
    MOD_LOADER_RUNTIME_CONTRACT_VERSION,
  );
  const phase = snapshot.phase;
  if (
    phase !== "missingLoader" &&
    phase !== "missingPayload" &&
    phase !== "stopped" &&
    phase !== "running"
  ) {
    throw new ModStudioContractError("phase is invalid");
  }
  const loaderPresent = boolean(snapshot.loaderPresent, "loaderPresent");
  const payloadPresent = boolean(snapshot.payloadPresent, "payloadPresent");
  const loaderFileName = string(snapshot.loaderFileName, "loaderFileName");
  const payloadRelativePath = string(
    snapshot.payloadRelativePath,
    "payloadRelativePath",
  );
  const placement = string(snapshot.placement, "placement");
  if (
    loaderFileName !== "nte-mod-loader.exe" ||
    payloadRelativePath !== "plugins/dwmapi.dll" ||
    placement !== "applicationDirectory"
  ) {
    throw new ModStudioContractError(
      "Mod Loader placement contract is invalid",
    );
  }
  if (
    (phase === "missingLoader" && loaderPresent) ||
    (phase === "missingPayload" && (!loaderPresent || payloadPresent)) ||
    ((phase === "stopped" || phase === "running") &&
      (!loaderPresent || !payloadPresent))
  ) {
    throw new ModStudioContractError(
      "Mod Loader runtime state is inconsistent",
    );
  }
  return {
    contractVersion,
    phase,
    loaderPresent,
    payloadPresent,
    loaderFileName,
    payloadRelativePath,
    placement,
  };
}

export function parseModMarketCatalog(
  value: unknown,
): ModMarketCatalogSnapshot {
  const catalog = record(value, "Mod Market catalog");
  const contractVersion = contractVersionOf(
    catalog,
    MOD_MARKET_CONTRACT_VERSION,
  );
  const privacyMode = string(catalog.privacyMode, "privacyMode");
  if (privacyMode !== "anonymous-read-only") {
    throw new ModStudioContractError("privacyMode is invalid");
  }
  const mods = array(catalog.mods, "mods");
  if (mods.length === 0 || mods.length > 64) {
    throw new ModStudioContractError("mods must contain 1-64 entries");
  }
  const parsedMods = mods.map((value, index) => {
    const item = record(value, `mods[${index}]`);
    const bindings = array(item.bindings, `mods[${index}].bindings`);
    if (bindings.length === 0 || bindings.length > 16) {
      throw new ModStudioContractError(
        `mods[${index}].bindings must contain 1-16 entries`,
      );
    }
    const parsedBindings = bindings.map((binding, bindingIndex) =>
      identifier(binding, `mods[${index}].bindings[${bindingIndex}]`, 31),
    );
    if (new Set(parsedBindings).size !== parsedBindings.length) {
      throw new ModStudioContractError(
        `mods[${index}].bindings contains duplicates`,
      );
    }
    const capabilities = array(
      item.capabilities,
      `mods[${index}].capabilities`,
    );
    if (capabilities.length > 16) {
      throw new ModStudioContractError(
        `mods[${index}].capabilities exceeds 16 entries`,
      );
    }
    return {
      id: modId(item.id, `mods[${index}].id`),
      bindings: parsedBindings,
      localizations: parseModMarketLocalizations(
        item.localizations,
        `mods[${index}].localizations`,
      ),
      version: semver(item.version, `mods[${index}].version`),
      author: boundedString(item.author, `mods[${index}].author`, 64),
      capabilities: capabilities.map((capability, capabilityIndex) =>
        identifier(
          capability,
          `mods[${index}].capabilities[${capabilityIndex}]`,
          64,
        ),
      ),
      packageSize: positiveInteger(
        item.packageSize,
        `mods[${index}].packageSize`,
      ),
      localState: parseModMarketLocalState(
        item.localState,
        `mods[${index}].localState`,
      ),
    };
  });
  const ids = new Set(parsedMods.map((item) => item.id));
  if (ids.size !== parsedMods.length) {
    throw new ModStudioContractError("mods contains duplicate Mod IDs");
  }
  return {
    contractVersion,
    publishedAt: boundedString(catalog.publishedAt, "publishedAt", 64),
    privacyMode,
    mods: parsedMods,
  };
}

function parseModMarketLocalState(
  value: unknown,
  field: string,
): ModMarketLocalState {
  const state = record(value, field);
  const status = string(state.status, `${field}.status`);
  switch (status) {
    case "notInstalled":
      exactFields(state, field, ["status"]);
      return { status };
    case "installed":
      exactFields(state, field, ["status", "enabled", "current"]);
      return {
        status,
        enabled: boolean(state.enabled, `${field}.enabled`),
        current: boolean(state.current, `${field}.current`),
      };
    case "unreadable":
      exactFields(state, field, ["status", "code", "messageKey"]);
      const code = modMarketUnreadableCode(state.code, `${field}.code`);
      const messageKey = boundedString(
        state.messageKey,
        `${field}.messageKey`,
        256,
      );
      const expectedMessageKey =
        MOD_MARKET_UNREADABLE_MESSAGE_KEY_BY_CODE[code];
      if (messageKey !== expectedMessageKey) {
        throw new ModStudioContractError(
          `${field}.messageKey does not match ${field}.code`,
        );
      }
      return {
        status,
        code,
        messageKey: expectedMessageKey,
      };
    default:
      throw new ModStudioContractError(`${field}.status is invalid`);
  }
}

function modMarketUnreadableCode(
  value: unknown,
  field: string,
): ModMarketUnreadableCode {
  const code = identifier(value, field, 64);
  if (
    !Object.prototype.hasOwnProperty.call(
      MOD_MARKET_UNREADABLE_MESSAGE_KEY_BY_CODE,
      code,
    )
  ) {
    throw new ModStudioContractError(`${field} is invalid`);
  }
  return code as ModMarketUnreadableCode;
}

function parseModMarketLocalizations(
  value: unknown,
  field: string,
): ModMarketItem["localizations"] {
  const localizations = record(value, field);
  return {
    en: parseModMarketLocalizedText(localizations.en, `${field}.en`),
    "zh-CN": parseModMarketLocalizedText(
      localizations["zh-CN"],
      `${field}.zh-CN`,
    ),
    ja: parseModMarketLocalizedText(localizations.ja, `${field}.ja`),
  };
}

function parseModMarketLocalizedText(
  value: unknown,
  field: string,
): ModMarketLocalizedText {
  const localized = record(value, field);
  return {
    name: boundedString(localized.name, `${field}.name`, 64),
    summary: boundedString(localized.summary, `${field}.summary`, 280),
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
    const status = enumValue(
      payload.status,
      [
        "connected",
        "loaderPresent",
        "waiting",
        "acknowledgementRequired",
        "probeFailed",
        "bootstrapFailed",
      ] as const,
      "payload.status",
    );
    const bootstrapErrorCode = nullableEnumValue(
      payload.bootstrapErrorCode,
      MOD_STUDIO_BOOTSTRAP_ERROR_CODES,
      "payload.bootstrapErrorCode",
    );
    const probeErrorCode = nullableEnumValue(
      payload.probeErrorCode,
      MOD_STUDIO_RUNTIME_PROBE_ERROR_CODES,
      "payload.probeErrorCode",
    );
    const probeOsErrorCode =
      payload.probeOsErrorCode === null
        ? null
        : nonNegativeInteger(
            payload.probeOsErrorCode,
            "payload.probeOsErrorCode",
          );
    if ((status === "bootstrapFailed") !== (bootstrapErrorCode !== null)) {
      throw new ModStudioContractError(
        "payload.bootstrapErrorCode must be present only for bootstrapFailed",
      );
    }
    if ((status === "probeFailed") !== (probeErrorCode !== null)) {
      throw new ModStudioContractError(
        "payload.probeErrorCode must be present only for probeFailed",
      );
    }
    if (status !== "probeFailed" && probeOsErrorCode !== null) {
      throw new ModStudioContractError(
        "payload.probeOsErrorCode must be present only for probeFailed",
      );
    }
    return {
      event: "connection",
      payload: {
        contractVersion,
        generation,
        status,
        bootstrapErrorCode,
        probeErrorCode,
        probeOsErrorCode,
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
  try {
    return parseStreamSubscriptionReceipt(value);
  } catch (error) {
    throw new ModStudioContractError(
      error instanceof Error
        ? error.message
        : "Invalid Mod runtime subscription receipt",
    );
  }
}

export function compareModStudioSequence(left: string, right: string): number {
  if (left.length !== right.length) {
    return left.length - right.length;
  }
  return left === right ? 0 : left < right ? -1 : 1;
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

function contractVersionOf(
  value: Record<string, unknown>,
  expectedVersion = MOD_STUDIO_CONTRACT_VERSION,
): number {
  const contractVersion = nonNegativeInteger(
    value.contractVersion,
    "contractVersion",
  );
  if (contractVersion !== expectedVersion) {
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

function semver(value: unknown, field: string): string {
  const parsed = string(value, field);
  if (!isCanonicalSemver(parsed)) {
    throw new ModStudioContractError(`${field} must be a semantic version`);
  }
  return parsed;
}

function identifier(value: unknown, field: string, maxLength: number): string {
  const parsed = string(value, field);
  if (
    parsed.length === 0 ||
    parsed.length > maxLength ||
    !/^[a-z0-9._-]+$/.test(parsed)
  ) {
    throw new ModStudioContractError(`${field} must be a valid identifier`);
  }
  return parsed;
}

function gameRegion(value: unknown, field: string): ModStudioGameRegion {
  const parsed = string(value, field);
  if (parsed !== "china" && parsed !== "global") {
    throw new ModStudioContractError(`${field} is invalid`);
  }
  return parsed;
}
