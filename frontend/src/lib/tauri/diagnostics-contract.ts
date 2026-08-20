import { createContractPrimitives } from "@/lib/tauri/contract-primitives";
import {
  parseTechnicalCommandError,
  TechnicalContractError,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";

export const DIAGNOSTICS_CONTRACT_VERSION = 3;
export const DIAGNOSTICS_MAX_CHECKS = 64;

const {
  array,
  boolean,
  boundedInteger,
  boundedStringAllowEmpty: text,
  canonicalDecimalString128: decimalString,
  enumValue,
  finiteNumber,
  integer,
  nonNegativeNumber,
  nonNegativeInteger,
  nullableBoundedStringAllowEmpty: nullableText,
  record: object,
} = createContractPrimitives((message) => {
  throw new TechnicalContractError(message);
});

export type DiagnosticsCommandError = TechnicalCommandError;
export type DiagnosticsStatus = "passed" | "warning" | "failed";
export type DiagnosticsCapturePhase =
  "idle" | "starting" | "running" | "stopping" | "stopped" | "failed";
export type DiagnosticsQualitySource =
  "live" | "pcapng_replay" | "json_replay" | "unknown";

export interface DiagnosticsMessageSnapshot {
  messageKey: string;
  messageArguments: string[];
}

export interface DiagnosticsCheckSnapshot {
  status: DiagnosticsStatus;
  titleKey: string;
  detail: DiagnosticsMessageSnapshot;
  suggestion: DiagnosticsMessageSnapshot;
}

export interface DiagnosticsReportSnapshot {
  failedCount: number;
  warningCount: number;
  checks: DiagnosticsCheckSnapshot[];
}

export interface DiagnosticsGameConnectionSnapshot {
  pid: number;
  localIp: string;
  remoteIp: string;
  remotePort: number;
}

export interface DiagnosticsEnvironmentSnapshot {
  deviceLabel: string | null;
  manualDevice: boolean;
  localIp: string | null;
  gameConnection: DiagnosticsGameConnectionSnapshot | null;
}

export interface DiagnosticsRawCaptureSnapshot {
  fileName: string | null;
  packetCount: string;
  capturedBytes: string;
  writeError: boolean;
  writing: boolean;
}

export interface DiagnosticsCaptureSnapshot {
  phase: DiagnosticsCapturePhase;
  replayRunning: boolean;
  activeFilter: string | null;
  droppedHistoryArchives: string;
  rawCapture: DiagnosticsRawCaptureSnapshot | null;
}

export interface DiagnosticsQualitySnapshot {
  source: DiagnosticsQualitySource;
  packetCount: number;
  packetsWithHits: number;
  hitCount: number;
  outgoingHits: string;
  outgoingDamage: number;
  unknownDirectionHits: string;
  unknownDirectionDamage: number;
  incomingHits: string;
  incomingDamage: number;
  unknownCharacterCount: number;
  unknownCharacterHits: string;
  unmappedSkillRows: number;
  unmappedSkillHits: string;
  unmappedGameplayEffectCount: number;
  timeStopEventCount: string;
  timeStopIntervalCount: number;
  abyssEventCount: string;
  serverDamageCorrections: string;
  unattributedServerDamageEvents: string;
  unattributedServerDamage: number;
}

export interface DiagnosticsActionsSnapshot {
  canImport: boolean;
  canExportParsed: boolean;
  canExportRaw: boolean;
}

export interface DiagnosticsSnapshot {
  contractVersion: number;
  captureGeneration: string;
  qualityGeneration: string;
  reportGeneration: string;
  adapterVersion: string;
  capture: DiagnosticsCaptureSnapshot;
  environment: DiagnosticsEnvironmentSnapshot | null;
  report: DiagnosticsReportSnapshot | null;
  quality: DiagnosticsQualitySnapshot;
  actions: DiagnosticsActionsSnapshot;
}

export interface DiagnosticsActionResult {
  performed: boolean;
  snapshot: DiagnosticsSnapshot;
}

export function parseDiagnosticsSnapshot(value: unknown): DiagnosticsSnapshot {
  const item = object(value, "diagnostics snapshot");
  const contractVersion = integer(
    item.contractVersion,
    "diagnostics.contractVersion",
  );
  if (contractVersion !== DIAGNOSTICS_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported diagnostics contract version: ${contractVersion}`,
    );
  }
  return {
    contractVersion,
    captureGeneration: decimalString(
      item.captureGeneration,
      "diagnostics.captureGeneration",
    ),
    qualityGeneration: decimalString(
      item.qualityGeneration,
      "diagnostics.qualityGeneration",
    ),
    reportGeneration: decimalString(
      item.reportGeneration,
      "diagnostics.reportGeneration",
    ),
    adapterVersion: text(
      item.adapterVersion,
      "diagnostics.adapterVersion",
      128,
    ),
    capture: parseCapture(item.capture),
    environment:
      item.environment === null
        ? null
        : parseEnvironment(item.environment, "diagnostics.environment"),
    report:
      item.report === null
        ? null
        : parseReport(item.report, "diagnostics.report"),
    quality: parseQuality(item.quality),
    actions: parseActions(item.actions),
  };
}

export function parseDiagnosticsActionResult(
  value: unknown,
): DiagnosticsActionResult {
  const item = object(value, "diagnostics action result");
  return {
    performed: boolean(item.performed, "diagnostics action performed"),
    snapshot: parseDiagnosticsSnapshot(item.snapshot),
  };
}

export function parseDiagnosticsEvent(value: unknown): DiagnosticsSnapshot {
  const item = object(value, "diagnostics event");
  if (item.event !== "snapshot") {
    throw new TechnicalContractError("Unsupported diagnostics event");
  }
  return parseDiagnosticsSnapshot(item.payload);
}

export function diagnosticsError(error: unknown): DiagnosticsCommandError {
  return parseTechnicalCommandError(error);
}

function parseCapture(value: unknown): DiagnosticsCaptureSnapshot {
  const item = object(value, "diagnostics.capture");
  return {
    phase: enumValue(
      item.phase,
      ["idle", "starting", "running", "stopping", "stopped", "failed"] as const,
      "diagnostics.capture.phase",
    ),
    replayRunning: boolean(
      item.replayRunning,
      "diagnostics.capture.replayRunning",
    ),
    activeFilter: nullableText(
      item.activeFilter,
      "diagnostics.capture.activeFilter",
      4_096,
    ),
    droppedHistoryArchives: decimalString(
      item.droppedHistoryArchives,
      "diagnostics.capture.droppedHistoryArchives",
    ),
    rawCapture:
      item.rawCapture === null
        ? null
        : parseRawCapture(item.rawCapture, "diagnostics.capture.rawCapture"),
  };
}

function parseRawCapture(
  value: unknown,
  field: string,
): DiagnosticsRawCaptureSnapshot {
  const item = object(value, field);
  return {
    fileName: nullableText(item.fileName, `${field}.fileName`, 1_024),
    packetCount: decimalString(item.packetCount, `${field}.packetCount`),
    capturedBytes: decimalString(item.capturedBytes, `${field}.capturedBytes`),
    writeError: boolean(item.writeError, `${field}.writeError`),
    writing: boolean(item.writing, `${field}.writing`),
  };
}

function parseEnvironment(
  value: unknown,
  field: string,
): DiagnosticsEnvironmentSnapshot {
  const item = object(value, field);
  return {
    deviceLabel: nullableText(item.deviceLabel, `${field}.deviceLabel`, 4_096),
    manualDevice: boolean(item.manualDevice, `${field}.manualDevice`),
    localIp: nullableText(item.localIp, `${field}.localIp`, 128),
    gameConnection:
      item.gameConnection === null
        ? null
        : parseGameConnection(item.gameConnection, `${field}.gameConnection`),
  };
}

function parseGameConnection(
  value: unknown,
  field: string,
): DiagnosticsGameConnectionSnapshot {
  const item = object(value, field);
  return {
    pid: nonNegativeInteger(item.pid, `${field}.pid`),
    localIp: text(item.localIp, `${field}.localIp`, 128),
    remoteIp: text(item.remoteIp, `${field}.remoteIp`, 128),
    remotePort: boundedInteger(
      item.remotePort,
      `${field}.remotePort`,
      0,
      65_535,
    ),
  };
}

function parseReport(value: unknown, field: string): DiagnosticsReportSnapshot {
  const item = object(value, field);
  const checks = array(item.checks, `${field}.checks`);
  if (checks.length > DIAGNOSTICS_MAX_CHECKS) {
    throw new TechnicalContractError(`${field}.checks exceeds UI bounds`);
  }
  return {
    failedCount: nonNegativeInteger(item.failedCount, `${field}.failedCount`),
    warningCount: nonNegativeInteger(
      item.warningCount,
      `${field}.warningCount`,
    ),
    checks: checks.map((check, index) =>
      parseCheck(check, `${field}.checks[${index}]`),
    ),
  };
}

function parseCheck(value: unknown, field: string): DiagnosticsCheckSnapshot {
  const item = object(value, field);
  return {
    status: enumValue(
      item.status,
      ["passed", "warning", "failed"] as const,
      `${field}.status`,
    ),
    titleKey: text(item.titleKey, `${field}.titleKey`, 1_024),
    detail: parseMessage(item.detail, `${field}.detail`),
    suggestion: parseMessage(item.suggestion, `${field}.suggestion`),
  };
}

function parseMessage(
  value: unknown,
  field: string,
): DiagnosticsMessageSnapshot {
  const item = object(value, field);
  const arguments_ = array(item.messageArguments, `${field}.messageArguments`);
  if (arguments_.length > 16) {
    throw new TechnicalContractError(
      `${field}.messageArguments exceeds bounds`,
    );
  }
  return {
    messageKey: text(item.messageKey, `${field}.messageKey`, 4_096),
    messageArguments: arguments_.map((argument, index) =>
      text(argument, `${field}.messageArguments[${index}]`, 16_384),
    ),
  };
}

function parseQuality(value: unknown): DiagnosticsQualitySnapshot {
  const item = object(value, "diagnostics.quality");
  return {
    source: enumValue(
      item.source,
      ["live", "pcapng_replay", "json_replay", "unknown"] as const,
      "diagnostics.quality.source",
    ),
    packetCount: nonNegativeInteger(
      item.packetCount,
      "diagnostics.quality.packetCount",
    ),
    packetsWithHits: nonNegativeInteger(
      item.packetsWithHits,
      "diagnostics.quality.packetsWithHits",
    ),
    hitCount: nonNegativeInteger(item.hitCount, "diagnostics.quality.hitCount"),
    outgoingHits: decimalString(
      item.outgoingHits,
      "diagnostics.quality.outgoingHits",
    ),
    outgoingDamage: finiteNumber(
      item.outgoingDamage,
      "diagnostics.quality.outgoingDamage",
    ),
    unknownDirectionHits: decimalString(
      item.unknownDirectionHits,
      "diagnostics.quality.unknownDirectionHits",
    ),
    unknownDirectionDamage: finiteNumber(
      item.unknownDirectionDamage,
      "diagnostics.quality.unknownDirectionDamage",
    ),
    incomingHits: decimalString(
      item.incomingHits,
      "diagnostics.quality.incomingHits",
    ),
    incomingDamage: finiteNumber(
      item.incomingDamage,
      "diagnostics.quality.incomingDamage",
    ),
    unknownCharacterCount: nonNegativeInteger(
      item.unknownCharacterCount,
      "diagnostics.quality.unknownCharacterCount",
    ),
    unknownCharacterHits: decimalString(
      item.unknownCharacterHits,
      "diagnostics.quality.unknownCharacterHits",
    ),
    unmappedSkillRows: nonNegativeInteger(
      item.unmappedSkillRows,
      "diagnostics.quality.unmappedSkillRows",
    ),
    unmappedSkillHits: decimalString(
      item.unmappedSkillHits,
      "diagnostics.quality.unmappedSkillHits",
    ),
    unmappedGameplayEffectCount: nonNegativeInteger(
      item.unmappedGameplayEffectCount,
      "diagnostics.quality.unmappedGameplayEffectCount",
    ),
    timeStopEventCount: decimalString(
      item.timeStopEventCount,
      "diagnostics.quality.timeStopEventCount",
    ),
    timeStopIntervalCount: nonNegativeInteger(
      item.timeStopIntervalCount,
      "diagnostics.quality.timeStopIntervalCount",
    ),
    abyssEventCount: decimalString(
      item.abyssEventCount,
      "diagnostics.quality.abyssEventCount",
    ),
    serverDamageCorrections: decimalString(
      item.serverDamageCorrections,
      "diagnostics.quality.serverDamageCorrections",
    ),
    unattributedServerDamageEvents: decimalString(
      item.unattributedServerDamageEvents,
      "diagnostics.quality.unattributedServerDamageEvents",
    ),
    unattributedServerDamage: nonNegativeNumber(
      item.unattributedServerDamage,
      "diagnostics.quality.unattributedServerDamage",
    ),
  };
}

function parseActions(value: unknown): DiagnosticsActionsSnapshot {
  const item = object(value, "diagnostics.actions");
  return {
    canImport: boolean(item.canImport, "diagnostics.actions.canImport"),
    canExportParsed: boolean(
      item.canExportParsed,
      "diagnostics.actions.canExportParsed",
    ),
    canExportRaw: boolean(
      item.canExportRaw,
      "diagnostics.actions.canExportRaw",
    ),
  };
}
