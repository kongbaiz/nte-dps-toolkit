import {
  createContractPrimitives,
  isCanonicalSemver,
} from "@/lib/tauri/contract-primitives";

export const MOD_MARKET_CONTRACT_VERSION = 13;
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
  positiveInteger,
  record,
  string,
} = createContractPrimitives((message) => {
  throw new ModStudioContractError(message);
});

export interface ModMarketItem {
  component: "plugin" | "host" | "loader" | "driver";
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
  | { status: "installed"; enabled: boolean | null; current: boolean }
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

export interface ModStudioCommandError {
  code: string;
  messageKey: string;
  messageArguments: string[];
  diagnosticLine: number | null;
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
      component: enumValue(
        item.component,
        ["plugin", "host", "loader", "driver"] as const,
        `mods[${index}].component`,
      ),
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
        enabled:
          state.enabled === null
            ? null
            : boolean(state.enabled, `${field}.enabled`),
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

function contractVersionOf(
  value: Record<string, unknown>,
  expectedVersion: number,
): number {
  const contractVersion = nonNegativeInteger(
    value.contractVersion,
    "contractVersion",
  );
  if (contractVersion !== expectedVersion) {
    throw new ModStudioContractError(
      `Unsupported Mod Market contract version: ${contractVersion}`,
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
