import {
  HUD_MODULE_IDS,
  parseHudConfigSnapshot,
  TechnicalContractError,
  type HudConfigSnapshot,
  type HudModuleId,
  type TechnicalCommandError,
} from "@/lib/tauri/technical-contract";
import { compareDecimalStrings } from "@/lib/decimal-string";

export const SETTINGS_CONTRACT_VERSION = 4;
export const HUD_SETTING_OPTION_IDS = [
  "title",
  "team_dps",
  "duration",
  "total_damage",
  "damage_taken",
  "character_rows",
  "abyss_half",
  "passthrough_state",
  "mini_timeline",
] as const;
export const HUD_PRESET_IDS = ["minimal", "standard", "detailed"] as const;
export const GLOBAL_HOTKEY_ACTION_IDS = ["capture", "reset", "hud"] as const;
export const LAYOUT_PROFILE_IDS = ["combat", "review", "research"] as const;

export type HudSettingOptionId = (typeof HUD_SETTING_OPTION_IDS)[number];
export type HudPresetId = (typeof HUD_PRESET_IDS)[number];
export type GlobalHotkeyActionId = (typeof GLOBAL_HOTKEY_ACTION_IDS)[number];
export type LayoutProfileId = (typeof LAYOUT_PROFILE_IDS)[number];
export type SettingsCommandError = TechnicalCommandError;
export type SettingsLanguage = "en" | "ja" | "zh-CN";
export type ThemePresetId = "zinc" | "tactical" | "high-contrast";
export type AccentId = "zinc" | "blue" | "violet" | "orange" | "green";
export type DensityId = "compact" | "cozy" | "comfortable";
export type DpsTimeModeId = "time-stop-adjusted" | "real-time";
export type PassthroughHotkeyId = "home" | "insert" | "f8" | "f9";
export type UpdateComponentId = "app" | "mods-plugin";
export type FunctionKey =
  | "F1"
  | "F2"
  | "F3"
  | "F4"
  | "F5"
  | "F6"
  | "F7"
  | "F8"
  | "F9"
  | "F10"
  | "F11"
  | "F12";

export interface InterfaceSettings {
  language: SettingsLanguage;
  darkMode: boolean;
  themePreset: ThemePresetId;
  accent: AccentId;
  density: DensityId;
  reduceMotion: boolean;
  islandNotifications: boolean;
  islandOffsetX: number;
}

export interface UpdateSettings {
  currentVersion: string;
  autoCheck: boolean;
  autoDownload: boolean;
  status: string;
  messageKey: string;
  messageArguments: string[];
  available: AvailableUpdate[];
  activeComponent: UpdateComponentId | null;
  downloadedBytes: string;
  totalBytes: string;
  prepared: PreparedUpdate | null;
  installEnabled: boolean;
  installBlockedMessageKey: string | null;
}

export interface AvailableUpdate {
  component: UpdateComponentId;
  version: string;
  publishedAt: string;
  notes: string;
  artifactSize: string;
}

export interface PreparedUpdate {
  component: UpdateComponentId;
  version: string;
}

export interface CaptureDevice {
  id: string;
  label: string;
}

export interface CaptureSettings {
  bpfFilter: string;
  devices: CaptureDevice[];
  manualCaptureDevice: string | null;
  serverDamageCalibration: boolean;
  separateReactionDamage: boolean;
  autoRoundAfterIdle: boolean;
  autoRoundIdleSeconds: number;
  autoRoundIdleSecondsMin: number;
  autoRoundIdleSecondsMax: number;
  dpsTimeMode: DpsTimeModeId;
  passthroughHotkey: PassthroughHotkeyId;
}

export interface HotkeyBinding {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  key: FunctionKey;
}

export interface GlobalHotkeyBinding {
  action: GlobalHotkeyActionId;
  binding: HotkeyBinding | null;
}

export interface GlobalHotkeys {
  enabled: boolean;
  bindings: GlobalHotkeyBinding[];
}

export interface CaptureFiles {
  count: number;
  totalBytes: string;
  formattedSize: string;
}

export interface TeamDataSettings {
  upperImported: boolean;
  lowerImported: boolean;
}

export interface TeamDataImportFileResult {
  performed: boolean;
  settings: SettingsSnapshot;
}

export interface SettingsSnapshot {
  contractVersion: number;
  generation: string;
  adapterVersion: string;
  interface: InterfaceSettings;
  updates: UpdateSettings;
  capture: CaptureSettings;
  hotkeys: GlobalHotkeys;
  captureFiles: CaptureFiles;
  teamData: TeamDataSettings;
  alwaysOnTop: boolean;
  hudWidthMin: number;
  hudWidthMax: number;
  hud: HudConfigSnapshot;
}

export interface SettingsSnapshotEvent {
  event: "snapshot";
  payload: SettingsSnapshot;
}

export type SettingsEvent = SettingsSnapshotEvent;

export type InterfaceSettingsInput = InterfaceSettings;
export interface UpdateSettingsInput {
  autoCheck: boolean;
  autoDownload: boolean;
}
export type CaptureSettingsInput = Omit<
  CaptureSettings,
  "devices" | "autoRoundIdleSecondsMin" | "autoRoundIdleSecondsMax"
>;

export function parseSettingsSnapshot(value: unknown): SettingsSnapshot {
  const snapshot = record(value, "settings snapshot");
  const contractVersion = integer(
    snapshot.contractVersion,
    "settings.contractVersion",
  );
  if (contractVersion !== SETTINGS_CONTRACT_VERSION) {
    throw new TechnicalContractError(
      `Unsupported settings contract version: ${contractVersion}`,
    );
  }

  const hudWidthMin = positiveInteger(
    snapshot.hudWidthMin,
    "settings.hudWidthMin",
  );
  const hudWidthMax = positiveInteger(
    snapshot.hudWidthMax,
    "settings.hudWidthMax",
  );
  if (hudWidthMin > hudWidthMax) {
    throw new TechnicalContractError(
      "settings.hudWidthMin must not exceed hudWidthMax",
    );
  }
  const hud = parseHudConfigSnapshot(snapshot.hud, "settings.hud");
  if (hud.width < hudWidthMin || hud.width > hudWidthMax) {
    throw new TechnicalContractError(
      "settings.hud.width must be inside the advertised bounds",
    );
  }

  const interface_ = record(snapshot.interface, "settings.interface");
  const updates = record(snapshot.updates, "settings.updates");
  const capture = record(snapshot.capture, "settings.capture");
  const hotkeys = record(snapshot.hotkeys, "settings.hotkeys");
  const captureFiles = record(snapshot.captureFiles, "settings.captureFiles");
  const teamData = record(snapshot.teamData, "settings.teamData");
  const idleMin = positiveInteger(
    capture.autoRoundIdleSecondsMin,
    "settings.capture.autoRoundIdleSecondsMin",
  );
  const idleMax = positiveInteger(
    capture.autoRoundIdleSecondsMax,
    "settings.capture.autoRoundIdleSecondsMax",
  );
  const idle = positiveInteger(
    capture.autoRoundIdleSeconds,
    "settings.capture.autoRoundIdleSeconds",
  );
  if (idleMin > idleMax || idle < idleMin || idle > idleMax) {
    throw new TechnicalContractError(
      "settings.capture.autoRoundIdleSeconds must be inside the advertised bounds",
    );
  }

  return {
    contractVersion,
    generation: decimalString(snapshot.generation, "settings.generation"),
    adapterVersion: string(snapshot.adapterVersion, "settings.adapterVersion"),
    interface: {
      language: enumValue(
        interface_.language,
        ["en", "ja", "zh-CN"] as const,
        "settings.interface.language",
      ),
      darkMode: boolean(interface_.darkMode, "settings.interface.darkMode"),
      themePreset: enumValue(
        interface_.themePreset,
        ["zinc", "tactical", "high-contrast"] as const,
        "settings.interface.themePreset",
      ),
      accent: enumValue(
        interface_.accent,
        ["zinc", "blue", "violet", "orange", "green"] as const,
        "settings.interface.accent",
      ),
      density: enumValue(
        interface_.density,
        ["compact", "cozy", "comfortable"] as const,
        "settings.interface.density",
      ),
      reduceMotion: boolean(
        interface_.reduceMotion,
        "settings.interface.reduceMotion",
      ),
      islandNotifications: boolean(
        interface_.islandNotifications,
        "settings.interface.islandNotifications",
      ),
      islandOffsetX: finiteNumber(
        interface_.islandOffsetX,
        "settings.interface.islandOffsetX",
      ),
    },
    updates: parseUpdateSettings(updates),
    capture: {
      bpfFilter: string(capture.bpfFilter, "settings.capture.bpfFilter"),
      devices: array(capture.devices, "settings.capture.devices").map(
        (device, index) => {
          const parsed = record(device, `settings.capture.devices[${index}]`);
          return {
            id: string(parsed.id, `settings.capture.devices[${index}].id`),
            label: string(
              parsed.label,
              `settings.capture.devices[${index}].label`,
            ),
          };
        },
      ),
      manualCaptureDevice: nullableString(
        capture.manualCaptureDevice,
        "settings.capture.manualCaptureDevice",
      ),
      serverDamageCalibration: boolean(
        capture.serverDamageCalibration,
        "settings.capture.serverDamageCalibration",
      ),
      separateReactionDamage: boolean(
        capture.separateReactionDamage,
        "settings.capture.separateReactionDamage",
      ),
      autoRoundAfterIdle: boolean(
        capture.autoRoundAfterIdle,
        "settings.capture.autoRoundAfterIdle",
      ),
      autoRoundIdleSeconds: idle,
      autoRoundIdleSecondsMin: idleMin,
      autoRoundIdleSecondsMax: idleMax,
      dpsTimeMode: enumValue(
        capture.dpsTimeMode,
        ["time-stop-adjusted", "real-time"] as const,
        "settings.capture.dpsTimeMode",
      ),
      passthroughHotkey: enumValue(
        capture.passthroughHotkey,
        ["home", "insert", "f8", "f9"] as const,
        "settings.capture.passthroughHotkey",
      ),
    },
    hotkeys: {
      enabled: boolean(hotkeys.enabled, "settings.hotkeys.enabled"),
      bindings: parseHotkeyBindings(hotkeys.bindings),
    },
    captureFiles: {
      count: nonNegativeInteger(
        captureFiles.count,
        "settings.captureFiles.count",
      ),
      totalBytes: decimalString(
        captureFiles.totalBytes,
        "settings.captureFiles.totalBytes",
      ),
      formattedSize: string(
        captureFiles.formattedSize,
        "settings.captureFiles.formattedSize",
      ),
    },
    teamData: {
      upperImported: boolean(
        teamData.upperImported,
        "settings.teamData.upperImported",
      ),
      lowerImported: boolean(
        teamData.lowerImported,
        "settings.teamData.lowerImported",
      ),
    },
    alwaysOnTop: boolean(snapshot.alwaysOnTop, "settings.alwaysOnTop"),
    hudWidthMin,
    hudWidthMax,
    hud,
  };
}

export function parseTeamDataImportFileResult(
  value: unknown,
): TeamDataImportFileResult {
  const result = record(value, "team data import result");
  return {
    performed: boolean(result.performed, "teamDataImport.performed"),
    settings: parseSettingsSnapshot(result.settings),
  };
}

function parseUpdateSettings(updates: Record<string, unknown>): UpdateSettings {
  const available = array(updates.available, "settings.updates.available").map(
    (value, index) => {
      const update = record(value, `settings.updates.available[${index}]`);
      return {
        component: enumValue(
          update.component,
          ["app", "mods-plugin"] as const,
          `settings.updates.available[${index}].component`,
        ),
        version: string(
          update.version,
          `settings.updates.available[${index}].version`,
        ),
        publishedAt: string(
          update.publishedAt,
          `settings.updates.available[${index}].publishedAt`,
        ),
        notes: string(
          update.notes,
          `settings.updates.available[${index}].notes`,
        ),
        artifactSize: decimalString(
          update.artifactSize,
          `settings.updates.available[${index}].artifactSize`,
        ),
      };
    },
  );
  if (
    available.length > 2 ||
    new Set(available.map((update) => update.component)).size !==
      available.length
  ) {
    throw new TechnicalContractError(
      "settings.updates.available must contain each stable component at most once",
    );
  }

  const activeComponent = nullableEnumValue(
    updates.activeComponent,
    ["app", "mods-plugin"] as const,
    "settings.updates.activeComponent",
  );
  const downloadedBytes = decimalString(
    updates.downloadedBytes,
    "settings.updates.downloadedBytes",
  );
  const totalBytes = decimalString(
    updates.totalBytes,
    "settings.updates.totalBytes",
  );
  if (compareSettingsGeneration(downloadedBytes, totalBytes) > 0) {
    throw new TechnicalContractError(
      "settings.updates.downloadedBytes must not exceed totalBytes",
    );
  }
  const prepared =
    updates.prepared === null
      ? null
      : parsePreparedUpdate(updates.prepared, "settings.updates.prepared");
  const installEnabled = boolean(
    updates.installEnabled,
    "settings.updates.installEnabled",
  );
  const installBlockedMessageKey = nullableString(
    updates.installBlockedMessageKey,
    "settings.updates.installBlockedMessageKey",
  );
  if (
    installEnabled &&
    (prepared === null || installBlockedMessageKey !== null)
  ) {
    throw new TechnicalContractError(
      "settings.updates install state is inconsistent",
    );
  }

  return {
    currentVersion: string(
      updates.currentVersion,
      "settings.updates.currentVersion",
    ),
    autoCheck: boolean(updates.autoCheck, "settings.updates.autoCheck"),
    autoDownload: boolean(
      updates.autoDownload,
      "settings.updates.autoDownload",
    ),
    status: string(updates.status, "settings.updates.status"),
    messageKey: string(updates.messageKey, "settings.updates.messageKey"),
    messageArguments: stringArray(
      updates.messageArguments,
      "settings.updates.messageArguments",
    ),
    available,
    activeComponent,
    downloadedBytes,
    totalBytes,
    prepared,
    installEnabled,
    installBlockedMessageKey,
  };
}

function parsePreparedUpdate(value: unknown, field: string): PreparedUpdate {
  const prepared = record(value, field);
  return {
    component: enumValue(
      prepared.component,
      ["app", "mods-plugin"] as const,
      `${field}.component`,
    ),
    version: string(prepared.version, `${field}.version`),
  };
}

function nullableEnumValue<const T extends readonly string[]>(
  value: unknown,
  options: T,
  field: string,
): T[number] | null {
  return value === null ? null : enumValue(value, options, field);
}

export function compareSettingsGeneration(left: string, right: string): number {
  return compareDecimalStrings(left, right);
}

export function parseSettingsEvent(value: unknown): SettingsEvent {
  const event = record(value, "settings event");
  if (event.event !== "snapshot") {
    throw new TechnicalContractError("Unknown settings event");
  }
  return {
    event: "snapshot",
    payload: parseSettingsSnapshot(event.payload),
  };
}

function parseHotkeyBindings(value: unknown): GlobalHotkeyBinding[] {
  const bindings = array(value, "settings.hotkeys.bindings").map(
    (binding, index) => {
      const item = record(binding, `settings.hotkeys.bindings[${index}]`);
      return {
        action: enumValue(
          item.action,
          GLOBAL_HOTKEY_ACTION_IDS,
          `settings.hotkeys.bindings[${index}].action`,
        ),
        binding:
          item.binding === null
            ? null
            : parseHotkeyBinding(
                item.binding,
                `settings.hotkeys.bindings[${index}].binding`,
              ),
      };
    },
  );
  if (
    bindings.length !== GLOBAL_HOTKEY_ACTION_IDS.length ||
    new Set(bindings.map((binding) => binding.action)).size !==
      GLOBAL_HOTKEY_ACTION_IDS.length
  ) {
    throw new TechnicalContractError(
      "settings.hotkeys.bindings must contain every stable action exactly once",
    );
  }
  return bindings;
}

function parseHotkeyBinding(value: unknown, field: string): HotkeyBinding {
  const binding = record(value, field);
  return {
    ctrl: boolean(binding.ctrl, `${field}.ctrl`),
    alt: boolean(binding.alt, `${field}.alt`),
    shift: boolean(binding.shift, `${field}.shift`),
    key: enumValue(
      binding.key,
      [
        "F1",
        "F2",
        "F3",
        "F4",
        "F5",
        "F6",
        "F7",
        "F8",
        "F9",
        "F10",
        "F11",
        "F12",
      ] as const,
      `${field}.key`,
    ),
  };
}

export function formatHotkeyBinding(
  binding: HotkeyBinding | null,
): string | null {
  if (binding === null) {
    return null;
  }
  return [
    binding.ctrl ? "Ctrl" : null,
    binding.alt ? "Alt" : null,
    binding.shift ? "Shift" : null,
    binding.key,
  ]
    .filter((part): part is string => part !== null)
    .join("+");
}

export function isHudModuleId(value: string): value is HudModuleId {
  return HUD_MODULE_IDS.includes(value as HudModuleId);
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

function nullableString(value: unknown, field: string): string | null {
  return value === null ? null : string(value, field);
}

function stringArray(value: unknown, field: string): string[] {
  return array(value, field).map((item, index) =>
    string(item, `${field}[${index}]`),
  );
}

function decimalString(value: unknown, field: string): string {
  const parsed = string(value, field);
  if (!/^(0|[1-9]\d*)$/.test(parsed)) {
    throw new TechnicalContractError(`${field} must be an unsigned decimal`);
  }
  return parsed;
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
    throw new TechnicalContractError(`${field} must not be negative`);
  }
  return parsed;
}

function positiveInteger(value: unknown, field: string): number {
  const parsed = integer(value, field);
  if (parsed <= 0) {
    throw new TechnicalContractError(`${field} must be greater than zero`);
  }
  return parsed;
}

function finiteNumber(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new TechnicalContractError(`${field} must be finite`);
  }
  return value;
}

function boolean(value: unknown, field: string): boolean {
  if (typeof value !== "boolean") {
    throw new TechnicalContractError(`${field} must be a boolean`);
  }
  return value;
}

function enumValue<const T extends readonly string[]>(
  value: unknown,
  values: T,
  field: string,
): T[number] {
  const parsed = string(value, field);
  if (!values.includes(parsed)) {
    throw new TechnicalContractError(`${field} has an unsupported value`);
  }
  return parsed as T[number];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
