export const CONSOLE_WINDOW_LABEL = "console";
export const MOD_STUDIO_CONTRACT_VERSION = 1;
export const MOD_STUDIO_MAX_DOCUMENTS = 256;
const MOD_ID_PATTERN = /^[a-z0-9._-]{1,31}$/;

export interface ModStudioDocumentSummary {
  id: string;
  enabled: boolean;
  sourceBytes: number;
  lineCount: number;
}

export interface ModStudioWorkspaceSnapshot {
  contractVersion: number;
  workspaceLabel: string;
  documents: ModStudioDocumentSummary[];
}

export interface ModStudioDocumentSnapshot {
  contractVersion: number;
  id: string;
  enabled: boolean;
  source: string;
}

export interface ModStudioCommandError {
  code: string;
  messageKey: string;
  messageArguments: string[];
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
    workspaceLabel: string(workspace.workspaceLabel, "workspaceLabel"),
    documents: parsedDocuments,
  };
}

export function parseModStudioDocument(
  value: unknown,
): ModStudioDocumentSnapshot {
  const document = record(value, "Mod document");
  return {
    contractVersion: contractVersionOf(document),
    id: modId(document.id, "id"),
    enabled: boolean(document.enabled, "enabled"),
    source: string(document.source, "source"),
  };
}

export function parseModStudioCommandError(
  value: unknown,
): ModStudioCommandError {
  const fallback: ModStudioCommandError = {
    code: "unexpected_mod_studio_error",
    messageKey: "The Mod workspace task stopped unexpectedly.",
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

export class ModStudioContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ModStudioContractError";
  }
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
