import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import {
  Check,
  ChartNoAxesCombined,
  Download,
  HardDrive,
  Languages,
  LayoutGrid,
  Palette,
  RefreshCw,
  SlidersHorizontal,
  Table2,
  Trash2,
  Upload,
  Users,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Markdown } from "@/components/ui/markdown";
import { Switch } from "@/components/ui/switch";
import { t, tf } from "@/lib/i18n";
import {
  type CaptureSettingsInput,
  type InterfaceSettingsInput,
  type LayoutProfileId,
  type MainDpsAttributionId,
  type MainDpsDisplayInput,
  type MainDpsMetricId,
  type SettingsSnapshot,
  type UpdateComponentId,
} from "@/lib/tauri/settings-contract";
import {
  formatByteCount,
  formatUpdateByteProgress,
  updateComponentLabelKey,
  updateProgressPercent,
} from "@/lib/update-presentation";
import { cn } from "@/lib/utils";

import { settingsSectionPending } from "./settings-view-model";

const CONTROL_CLASS_NAME =
  "h-8 w-full min-w-0 rounded-md border bg-background px-2.5 text-sm outline-none transition-colors focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50";

const LANGUAGE_OPTIONS = [
  { value: "zh-CN", label: "简体中文" },
  { value: "en", label: "English" },
  { value: "ja", label: "日本語" },
] as const;
const THEME_OPTIONS = [
  { value: "zinc", labelKey: "Zinc Theme" },
  { value: "tactical", labelKey: "Tactical" },
  { value: "high-contrast", labelKey: "High Contrast" },
] as const;
const ACCENT_OPTIONS = [
  { value: "zinc", labelKey: "Zinc" },
  { value: "blue", labelKey: "Blue" },
  { value: "violet", labelKey: "Violet" },
  { value: "orange", labelKey: "Orange" },
  { value: "green", labelKey: "Green" },
] as const;
const DENSITY_OPTIONS = [
  { value: "compact", labelKey: "Compact" },
  { value: "cozy", labelKey: "Cozy" },
  { value: "comfortable", labelKey: "Comfortable" },
] as const;
const DPS_TIME_OPTIONS = [
  { value: "time-stop-adjusted", labelKey: "Exclude Time Stop" },
  { value: "real-time", labelKey: "Real Time (incl. time stop)" },
] as const;
const LAYOUT_PROFILES = [
  {
    id: "combat",
    labelKey: "Combat Layout",
    descriptionKey: "Minimal HUD, compact density and mouse passthrough",
  },
  {
    id: "review",
    labelKey: "Review Layout",
    descriptionKey:
      "Normal window with Console timeline for post-combat review",
  },
  {
    id: "research",
    labelKey: "Research Layout",
    descriptionKey: "Console packets with team details for investigation",
  },
] as const;
const MAIN_DPS_METRICS: ReadonlyArray<{
  id: MainDpsMetricId;
  labelKey: string;
}> = [
  { id: "team-dps", labelKey: "Team DPS" },
  { id: "total-damage", labelKey: "Total Damage" },
  { id: "total-damage-taken", labelKey: "Total Damage Taken" },
  { id: "duration", labelKey: "Time" },
];
const MAIN_DPS_ATTRIBUTIONS: ReadonlyArray<{
  id: MainDpsAttributionId;
  labelKey: string;
}> = [
  { id: "character", labelKey: "Character attributed" },
  { id: "reaction", labelKey: "Reaction Damage" },
  { id: "shared", labelKey: "Shared mechanics" },
  { id: "unattributed", labelKey: "Unattributed" },
  { id: "max-hp-reduction", labelKey: "Life reduction" },
];
export interface SettingsCatalogActions {
  setInterface(settings: InterfaceSettingsInput): Promise<void>;
  setUpdatePreferences(
    autoCheck: boolean,
    autoDownload: boolean,
  ): Promise<void>;
  checkUpdates(): Promise<void>;
  downloadUpdate(component: UpdateComponentId): Promise<void>;
  installUpdate(): Promise<void>;
  setCapture(settings: CaptureSettingsInput): Promise<void>;
  setMainDpsDisplay(settings: MainDpsDisplayInput): Promise<void>;
  refreshCaptureDevices(): Promise<void>;
  applyLayoutProfile(profile: LayoutProfileId): Promise<void>;
  importTeamDataFile(): Promise<boolean | null>;
  exportTeamData(): Promise<boolean | null>;
  refreshCaptureFiles(): Promise<void>;
  clearCaptureFiles(): Promise<void>;
  openAbyssValues(): Promise<void>;
}

export function PrimarySettingsColumn({
  snapshot,
  pendingAction,
  actions,
}: {
  snapshot: SettingsSnapshot;
  pendingAction: string | null;
  actions: SettingsCatalogActions;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <InterfaceSettingsCard
        snapshot={snapshot}
        pending={settingsSectionPending(pendingAction, "interface")}
        actions={actions}
      />
      <SoftwareUpdateCard
        snapshot={snapshot}
        pending={settingsSectionPending(pendingAction, "update")}
        actions={actions}
      />
      <ParseSettingsCard
        snapshot={snapshot}
        pending={settingsSectionPending(pendingAction, "capture")}
        actions={actions}
      />
    </div>
  );
}

export function SecondarySettingsSections({
  snapshot,
  pendingAction,
  actions,
}: {
  snapshot: SettingsSnapshot;
  pendingAction: string | null;
  actions: SettingsCatalogActions;
}) {
  return (
    <>
      <LayoutProfilesCard
        pending={settingsSectionPending(pendingAction, "layout")}
        actions={actions}
      />
      <MainDpsDisplayCard
        snapshot={snapshot}
        pending={settingsSectionPending(pendingAction, "main-dps-display")}
        actions={actions}
      />
      <TeamDataCard
        snapshot={snapshot}
        pending={settingsSectionPending(pendingAction, "team-data")}
        actions={actions}
      />
      <CaptureFilesCard
        snapshot={snapshot}
        pending={settingsSectionPending(pendingAction, "capture-files")}
        actions={actions}
      />
      <AbyssValuesCard
        pending={settingsSectionPending(pendingAction, "abyss-values")}
        actions={actions}
      />
    </>
  );
}

function interfaceInput(snapshot: SettingsSnapshot): InterfaceSettingsInput {
  return { ...snapshot.interface };
}

function captureInput(snapshot: SettingsSnapshot): CaptureSettingsInput {
  const {
    devices: _devices,
    autoRoundIdleSecondsMin: _minimum,
    autoRoundIdleSecondsMax: _maximum,
    dpsTimeRuntime: _runtime,
    ...settings
  } = snapshot.capture;
  return settings;
}

function InterfaceSettingsCard({
  snapshot,
  pending,
  actions,
}: {
  snapshot: SettingsSnapshot;
  pending: boolean;
  actions: SettingsCatalogActions;
}) {
  const settings = interfaceInput(snapshot);
  const [islandOffset, setIslandOffset] = useState(
    String(settings.islandOffsetX),
  );
  useEffect(() => {
    setIslandOffset(String(settings.islandOffsetX));
  }, [settings.islandOffsetX]);
  const save = (patch: Partial<InterfaceSettingsInput>) =>
    actions.setInterface({ ...settings, ...patch });

  return (
    <SettingsCard icon={Languages} titleKey="Interface">
      <SettingsRow labelKey="Language">
        <NativeSelect
          disabled={pending}
          ariaLabel={t("Language")}
          value={settings.language}
          onChange={(language) =>
            void save({
              language: language as InterfaceSettingsInput["language"],
            })
          }
          options={LANGUAGE_OPTIONS.map((option) => ({ ...option }))}
        />
      </SettingsRow>
      <SettingsRow labelKey="Theme Preset">
        <NativeSelect
          disabled={pending}
          ariaLabel={t("Theme Preset")}
          value={settings.themePreset}
          onChange={(themePreset) =>
            void save({
              themePreset: themePreset as InterfaceSettingsInput["themePreset"],
            })
          }
          options={THEME_OPTIONS.map((option) => ({
            value: option.value,
            label: t(option.labelKey),
          }))}
        />
      </SettingsRow>
      <SettingsRow labelKey="Accent">
        <NativeSelect
          disabled={pending}
          ariaLabel={t("Accent")}
          value={settings.accent}
          onChange={(accent) =>
            void save({ accent: accent as InterfaceSettingsInput["accent"] })
          }
          options={ACCENT_OPTIONS.map((option) => ({
            value: option.value,
            label: t(option.labelKey),
          }))}
        />
      </SettingsRow>
      <SettingsRow labelKey="Density">
        <NativeSelect
          disabled={pending}
          ariaLabel={t("Density")}
          value={settings.density}
          onChange={(density) =>
            void save({ density: density as InterfaceSettingsInput["density"] })
          }
          options={DENSITY_OPTIONS.map((option) => ({
            value: option.value,
            label: t(option.labelKey),
          }))}
        />
      </SettingsRow>
      <SettingsRow labelKey="Motion">
        <LabeledSwitch
          disabled={pending}
          labelKey="Reduce motion"
          checked={settings.reduceMotion}
          onCheckedChange={(reduceMotion) => void save({ reduceMotion })}
        />
      </SettingsRow>
      <SettingsRow labelKey="Notifications">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <LabeledSwitch
            disabled={pending}
            labelKey="Floating island notifications"
            checked={settings.islandNotifications}
            onCheckedChange={(islandNotifications) =>
              void save({ islandNotifications })
            }
          />
          {settings.islandNotifications ? (
            <label className="flex items-center gap-2">
              <span className="text-xs text-muted-foreground">
                {t("Horizontal offset from the screen center")}
              </span>
              <input
                className={cn(CONTROL_CLASS_NAME, "w-20 text-right")}
                disabled={pending}
                type="number"
                value={islandOffset}
                onChange={(event) => setIslandOffset(event.target.value)}
                onBlur={() => {
                  const value = Number(islandOffset);
                  if (Number.isFinite(value)) {
                    void save({ islandOffsetX: value });
                  } else {
                    setIslandOffset(String(settings.islandOffsetX));
                  }
                }}
              />
              <span className="text-xs text-muted-foreground">px</span>
            </label>
          ) : null}
        </div>
      </SettingsRow>
    </SettingsCard>
  );
}

function SoftwareUpdateCard({
  snapshot,
  pending,
  actions,
}: {
  snapshot: SettingsSnapshot;
  pending: boolean;
  actions: SettingsCatalogActions;
}) {
  const update = snapshot.updates;
  const busy = ["checking", "downloading", "installing", "restarting"].includes(
    update.status,
  );
  const showProgress =
    update.status === "downloading" && update.activeComponent !== null;
  const progress = updateProgressPercent(
    update.downloadedBytes,
    update.totalBytes,
  );
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Download className="size-4" aria-hidden="true" />
          {t("Software Update")}
        </CardTitle>
        <CardDescription>
          {t(
            "Update connections follow the Windows system proxy, automatic proxy script, and direct-connection fallback",
          )}
        </CardDescription>
        <CardAction>
          <Badge variant="outline">
            {t("Current Version")} · {update.currentVersion}
          </Badge>
        </CardAction>
      </CardHeader>
      <CardContent className="divide-y">
        <SettingsSwitchRow
          disabled={pending}
          labelKey="Automatically check for official updates"
          checked={update.autoCheck}
          onCheckedChange={(autoCheck) =>
            void actions.setUpdatePreferences(autoCheck, update.autoDownload)
          }
        />
        <SettingsSwitchRow
          disabled={pending}
          labelKey="Automatically download verified updates"
          checked={update.autoDownload}
          onCheckedChange={(autoDownload) =>
            void actions.setUpdatePreferences(update.autoCheck, autoDownload)
          }
        />
        {update.available.map((available) => (
          <section
            className="flex flex-col gap-2.5 py-3"
            key={available.component}
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex min-w-0 items-center gap-2">
                <Badge variant="outline">
                  {t(updateComponentLabelKey(available.component))}
                </Badge>
                <strong className="text-sm">{available.version}</strong>
              </div>
              <span className="text-xs text-muted-foreground">
                {available.publishedAt} ·{" "}
                {formatByteCount(available.artifactSize)}
              </span>
            </div>
            {available.notes.trim() ? (
              <Markdown className="text-xs leading-relaxed text-muted-foreground">
                {available.notes}
              </Markdown>
            ) : null}
            <div>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={pending || busy || update.prepared !== null}
                onClick={() => void actions.downloadUpdate(available.component)}
              >
                <Download aria-hidden="true" />
                {t(
                  available.component === "app"
                    ? "Download application update"
                    : "Download Mod loader update",
                )}
              </Button>
            </div>
          </section>
        ))}
        {showProgress ? (
          <section className="flex flex-col gap-2 py-3">
            <div className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
              <span>{t(update.messageKey)}</span>
              <span>
                {formatUpdateByteProgress(
                  update.downloadedBytes,
                  update.totalBytes,
                )}
              </span>
            </div>
            <div
              className="h-2 overflow-hidden rounded-full bg-muted"
              role="progressbar"
              aria-label={t(update.messageKey)}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={progress ?? undefined}
              aria-valuetext={formatUpdateByteProgress(
                update.downloadedBytes,
                update.totalBytes,
              )}
            >
              <div
                className={cn(
                  "h-full rounded-full bg-primary transition-[width]",
                  progress === null && "animate-pulse",
                )}
                style={{ width: progress === null ? "35%" : `${progress}%` }}
              />
            </div>
          </section>
        ) : null}
        {update.prepared ? (
          <section className="flex flex-col gap-2.5 py-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <Badge>{t("Ready")}</Badge>
                <span className="text-sm font-medium">
                  {t(updateComponentLabelKey(update.prepared.component))} ·{" "}
                  {update.prepared.version}
                </span>
              </div>
              <Button
                type="button"
                size="sm"
                disabled={pending || !update.installEnabled}
                onClick={() => void actions.installUpdate()}
              >
                {t(
                  update.prepared.component === "app"
                    ? "Install and restart"
                    : "Install Mod loader update",
                )}
              </Button>
            </div>
            {update.installBlockedMessageKey ? (
              <p className="text-xs text-destructive">
                {t(update.installBlockedMessageKey)}
              </p>
            ) : null}
          </section>
        ) : null}
      </CardContent>
      <CardFooter className="flex-wrap justify-between gap-3">
        <span className="flex items-center gap-2 text-xs text-muted-foreground">
          <span
            className={cn(
              "size-1.5 rounded-full bg-muted-foreground",
              busy && "animate-pulse bg-primary",
              update.status === "error" && "bg-destructive",
            )}
          />
          {tf(update.messageKey, update.messageArguments)}
        </span>
        <Button
          type="button"
          variant="outline"
          disabled={pending || busy || update.prepared !== null}
          onClick={() => void actions.checkUpdates()}
        >
          <RefreshCw
            className={cn(update.status === "checking" && "animate-spin")}
            aria-hidden="true"
          />
          {t("Check for updates")}
        </Button>
      </CardFooter>
    </Card>
  );
}

function ParseSettingsCard({
  snapshot,
  pending,
  actions,
}: {
  snapshot: SettingsSnapshot;
  pending: boolean;
  actions: SettingsCatalogActions;
}) {
  const settings = captureInput(snapshot);
  const [filter, setFilter] = useState(settings.bpfFilter);
  const [idleSeconds, setIdleSeconds] = useState(
    String(settings.autoRoundIdleSeconds),
  );
  useEffect(() => setFilter(settings.bpfFilter), [settings.bpfFilter]);
  useEffect(
    () => setIdleSeconds(String(settings.autoRoundIdleSeconds)),
    [settings.autoRoundIdleSeconds],
  );
  const save = (patch: Partial<CaptureSettingsInput>) =>
    actions.setCapture({ ...settings, ...patch });
  const dpsTimeDescription =
    settings.dpsTimeMode === "real-time"
      ? "Output time accrues over the capture time span"
      : "Uses authoritative game pause intervals when the combat-clock provider is available";

  return (
    <SettingsCard icon={SlidersHorizontal} titleKey="Parse Settings">
      <SettingsRow
        labelKey="BPF Filter"
        descriptionKey="Capture filter expression; takes effect on the next capture"
      >
        <input
          className={CONTROL_CLASS_NAME}
          disabled={pending}
          aria-label={t("BPF Filter")}
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          onBlur={() => {
            const value = filter.trim();
            if (value) void save({ bpfFilter: value });
            else setFilter(settings.bpfFilter);
          }}
          onKeyDown={(event) =>
            submitOnEnter(event, () => event.currentTarget.blur())
          }
        />
      </SettingsRow>
      <SettingsRow labelKey="Capture NIC">
        <div className="flex min-w-0 flex-col gap-2">
          <div className="flex min-w-0 flex-wrap gap-2">
            <NativeSelect
              disabled={
                pending ||
                !snapshot.capture.devicesAvailable ||
                snapshot.capture.devices.length === 0
              }
              className="min-w-52 flex-1"
              ariaLabel={t("Capture NIC")}
              value={
                settings.manualCaptureDevice === null ? "automatic" : "manual"
              }
              onChange={(mode) =>
                void save({
                  manualCaptureDevice:
                    mode === "automatic"
                      ? null
                      : (snapshot.capture.devices[0]?.id ?? null),
                })
              }
              options={[
                { value: "automatic", label: t("Automatic NIC selection") },
                { value: "manual", label: t("Pin capture NIC") },
              ]}
            />
            {snapshot.capture.devicesAvailable &&
            settings.manualCaptureDevice !== null &&
            snapshot.capture.devices.length > 0 ? (
              <NativeSelect
                disabled={pending}
                className="min-w-52 flex-1"
                ariaLabel={t("Select a NIC")}
                value={settings.manualCaptureDevice}
                onChange={(manualCaptureDevice) =>
                  void save({ manualCaptureDevice })
                }
                options={[
                  ...(snapshot.capture.devices.some(
                    (device) => device.id === settings.manualCaptureDevice,
                  )
                    ? []
                    : [
                        {
                          value: settings.manualCaptureDevice,
                          label: settings.manualCaptureDevice,
                        },
                      ]),
                  ...snapshot.capture.devices.map((device) => ({
                    value: device.id,
                    label: device.label,
                  })),
                ]}
              />
            ) : null}
            <Button
              type="button"
              variant="outline"
              disabled={pending}
              onClick={() => void actions.refreshCaptureDevices()}
            >
              <RefreshCw aria-hidden="true" />
              {t("Refresh NIC List")}
            </Button>
          </div>
          {!snapshot.capture.devicesAvailable ? (
            <p className="text-xs leading-relaxed text-amber-600 dark:text-amber-300">
              {t("Capture devices are unavailable.")}
            </p>
          ) : snapshot.capture.devices.length === 0 ? (
            <p className="text-xs leading-relaxed text-amber-600 dark:text-amber-300">
              {t(
                "No usable NIC found; confirm Npcap is installed, then click refresh",
              )}
            </p>
          ) : settings.manualCaptureDevice !== null &&
            !snapshot.capture.devices.some(
              (device) => device.id === settings.manualCaptureDevice,
            ) ? (
            <p className="text-xs leading-relaxed text-amber-600 dark:text-amber-300">
              {t(
                "The selected NIC is currently unavailable; reselect or click refresh",
              )}
            </p>
          ) : null}
        </div>
      </SettingsRow>
      <SettingsRow
        labelKey="Damage Source"
        descriptionKey="When enabled, skills missing from the gameplay-effect semantics table use server settlements; listed skills keep their declared policy. Disabled mode reports unexplained residuals without changing DPS totals."
      >
        <LabeledSwitch
          disabled={pending}
          labelKey="Calibrate with server-side HP deltas"
          checked={settings.serverDamageCalibration}
          onCheckedChange={(serverDamageCalibration) =>
            void save({ serverDamageCalibration })
          }
        />
      </SettingsRow>
      <SettingsRow
        labelKey="Damage Total"
        descriptionKey="When enabled, maximum HP reduction is added to team total damage and DPS, and all attribution percentages use that combined total."
      >
        <LabeledSwitch
          disabled={pending}
          labelKey="Include maximum HP reduction in total damage"
          checked={settings.includeMaxHpReductionInTotalDamage}
          onCheckedChange={(includeMaxHpReductionInTotalDamage) =>
            void save({ includeMaxHpReductionInTotalDamage })
          }
        />
      </SettingsRow>
      <SettingsRow
        labelKey="Character Damage"
        descriptionKey="When enabled, confirmed reaction damage is shown separately instead of being added to the attributed character; team total damage is unchanged."
      >
        <LabeledSwitch
          disabled={pending}
          labelKey="Separate reaction damage from character damage"
          checked={settings.separateReactionDamage}
          onCheckedChange={(separateReactionDamage) =>
            void save({ separateReactionDamage })
          }
        />
      </SettingsRow>
      <SettingsRow
        labelKey="Combat Rounds"
        descriptionKey="Outside abyss, archive the current round and clear live combat stats after the configured idle time"
      >
        <div className="flex flex-wrap items-center justify-between gap-3">
          <LabeledSwitch
            disabled={pending}
            labelKey="Auto start a new round after idle"
            checked={settings.autoRoundAfterIdle}
            onCheckedChange={(autoRoundAfterIdle) =>
              void save({ autoRoundAfterIdle })
            }
          />
          {settings.autoRoundAfterIdle ? (
            <label className="flex items-center gap-2">
              <input
                className={cn(CONTROL_CLASS_NAME, "w-20 text-right")}
                disabled={pending}
                type="number"
                min={snapshot.capture.autoRoundIdleSecondsMin}
                max={snapshot.capture.autoRoundIdleSecondsMax}
                value={idleSeconds}
                onChange={(event) => setIdleSeconds(event.target.value)}
                onBlur={() => {
                  const value = Number(idleSeconds);
                  if (Number.isSafeInteger(value)) {
                    void save({ autoRoundIdleSeconds: value });
                  } else {
                    setIdleSeconds(String(settings.autoRoundIdleSeconds));
                  }
                }}
              />
              <span className="text-xs text-muted-foreground">s</span>
            </label>
          ) : null}
        </div>
      </SettingsRow>
      <SettingsRow labelKey="DPS Time" descriptionKey={dpsTimeDescription}>
        <div className="flex min-w-0 flex-col gap-2">
          <NativeSelect
            disabled={pending}
            ariaLabel={t("DPS Time")}
            value={settings.dpsTimeMode}
            onChange={(dpsTimeMode) =>
              void save({
                dpsTimeMode: dpsTimeMode as CaptureSettingsInput["dpsTimeMode"],
              })
            }
            options={DPS_TIME_OPTIONS.map((option) => ({
              value: option.value,
              label: t(option.labelKey),
            }))}
          />
          {settings.dpsTimeMode === "time-stop-adjusted" &&
          snapshot.capture.dpsTimeRuntime.warningMessageKey ? (
            <p
              className="text-xs leading-relaxed text-amber-600 dark:text-amber-300"
              role="status"
            >
              {t(snapshot.capture.dpsTimeRuntime.warningMessageKey)}
            </p>
          ) : null}
        </div>
      </SettingsRow>
    </SettingsCard>
  );
}

function LayoutProfilesCard({
  pending,
  actions,
}: {
  pending: boolean;
  actions: SettingsCatalogActions;
}) {
  return (
    <SettingsCard icon={LayoutGrid} titleKey="Layout Profiles">
      {LAYOUT_PROFILES.map((profile) => (
        <div
          className="grid gap-2 py-2.5 first:pt-0 last:pb-0 sm:grid-cols-[9rem_minmax(0,1fr)] sm:items-center"
          key={profile.id}
        >
          <Button
            type="button"
            variant="outline"
            disabled={pending}
            onClick={() =>
              void actions.applyLayoutProfile(profile.id as LayoutProfileId)
            }
          >
            {t(profile.labelKey)}
          </Button>
          <p className="text-xs leading-relaxed text-muted-foreground">
            {t(profile.descriptionKey)}
          </p>
        </div>
      ))}
    </SettingsCard>
  );
}

function MainDpsDisplayCard({
  snapshot,
  pending,
  actions,
}: {
  snapshot: SettingsSnapshot;
  pending: boolean;
  actions: SettingsCatalogActions;
}) {
  const display = snapshot.mainDps;
  return (
    <SettingsCard icon={ChartNoAxesCombined} titleKey="Main DPS display">
      <SettingsRow
        labelKey="Value modules"
        descriptionKey="Choose the statistics shown on the main DPS page"
      >
        <div className="grid gap-2 sm:grid-cols-2">
          {MAIN_DPS_METRICS.map((metric) => (
            <LabeledSwitch
              key={metric.id}
              disabled={pending}
              labelKey={metric.labelKey}
              checked={display.metrics.includes(metric.id)}
              onCheckedChange={(checked) =>
                void actions.setMainDpsDisplay({
                  ...display,
                  metrics: withSelection(display.metrics, metric.id, checked),
                })
              }
            />
          ))}
        </div>
      </SettingsRow>
      <SettingsRow
        labelKey="Damage attribution types"
        descriptionKey="Choose the damage attribution chips shown on the main DPS page"
      >
        <div className="grid gap-2 sm:grid-cols-2">
          {MAIN_DPS_ATTRIBUTIONS.map((attribution) => (
            <LabeledSwitch
              key={attribution.id}
              disabled={pending}
              labelKey={attribution.labelKey}
              checked={display.attributions.includes(attribution.id)}
              onCheckedChange={(checked) =>
                void actions.setMainDpsDisplay({
                  ...display,
                  attributions: withSelection(
                    display.attributions,
                    attribution.id,
                    checked,
                  ),
                })
              }
            />
          ))}
        </div>
      </SettingsRow>
    </SettingsCard>
  );
}

function withSelection<T extends string>(
  values: readonly T[],
  value: T,
  selected: boolean,
): T[] {
  if (selected)
    return values.includes(value) ? [...values] : [...values, value];
  return values.filter((candidate) => candidate !== value);
}

function TeamDataCard({
  snapshot,
  pending,
  actions,
}: {
  snapshot: SettingsSnapshot;
  pending: boolean;
  actions: SettingsCatalogActions;
}) {
  const exportedTimer = useRef<number | null>(null);
  const [exported, setExported] = useState(false);
  const imported =
    snapshot.teamData.upperImported || snapshot.teamData.lowerImported;
  useEffect(
    () => () => {
      if (exportedTimer.current !== null) {
        window.clearTimeout(exportedTimer.current);
      }
    },
    [],
  );

  const exportTeamData = async () => {
    setExported(false);
    const saved = await actions.exportTeamData();
    if (saved !== true) return;
    setExported(true);
    if (exportedTimer.current !== null) {
      window.clearTimeout(exportedTimer.current);
    }
    exportedTimer.current = window.setTimeout(() => {
      setExported(false);
      exportedTimer.current = null;
    }, 2400);
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Users className="size-4" aria-hidden="true" />
          {t("Team Data")}
        </CardTitle>
        {imported ? (
          <CardAction>
            <Badge variant="outline">{t("Imported")}</Badge>
          </CardAction>
        ) : null}
      </CardHeader>
      <CardContent className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="outline"
          disabled={pending || !snapshot.teamData.available}
          onClick={() => void actions.importTeamDataFile()}
        >
          <Upload aria-hidden="true" />
          {t("Import DPS Data")}
        </Button>
        <Button
          type="button"
          variant="outline"
          disabled={pending || !snapshot.teamData.available}
          onClick={() => void exportTeamData()}
        >
          {exported ? (
            <Check aria-hidden="true" />
          ) : (
            <Download aria-hidden="true" />
          )}
          {t("Export Team Data")}
        </Button>
        <span
          aria-live="polite"
          className={cn(
            "self-center text-xs text-emerald-600 transition-opacity",
            exported ? "opacity-100" : "pointer-events-none opacity-0",
          )}
        >
          {t("Team data exported")}
        </span>
      </CardContent>
      <CardFooter className="text-xs text-muted-foreground">
        {snapshot.teamData.available
          ? t(
              "Import/export is scene-independent; works in both open world and abyss",
            )
          : t("Imported team data is unavailable.")}
      </CardFooter>
    </Card>
  );
}

function CaptureFilesCard({
  snapshot,
  pending,
  actions,
}: {
  snapshot: SettingsSnapshot;
  pending: boolean;
  actions: SettingsCatalogActions;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <HardDrive className="size-4" aria-hidden="true" />
          {t("Capture Files")}
        </CardTitle>
        <CardDescription>
          {replaceSequential(t("Raw captures: {} · {}"), [
            String(snapshot.captureFiles.count),
            snapshot.captureFiles.formattedSize,
          ])}
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="outline"
          disabled={pending}
          onClick={() => void actions.refreshCaptureFiles()}
        >
          <RefreshCw aria-hidden="true" />
          {t("Refresh")}
        </Button>
        <Button
          type="button"
          variant="destructive"
          disabled={pending || snapshot.captureFiles.count === 0}
          onClick={() => void actions.clearCaptureFiles()}
        >
          <Trash2 aria-hidden="true" />
          {t("Clear")}
        </Button>
      </CardContent>
      <CardFooter className="text-xs leading-relaxed text-muted-foreground">
        {t(
          "Live capture writes raw frames to logs/nte_raw_*.pcapng; clearing does not affect stats or history.",
        )}
      </CardFooter>
    </Card>
  );
}

function AbyssValuesCard({
  pending,
  actions,
}: {
  pending: boolean;
  actions: SettingsCatalogActions;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Table2 className="size-4" aria-hidden="true" />
          {t("Abyss Values")}
        </CardTitle>
        <CardDescription>
          {t(
            "Opens in a separate window so you can view it side by side with live DPS",
          )}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <Button
          type="button"
          variant="outline"
          disabled={pending}
          onClick={() => void actions.openAbyssValues()}
        >
          <Table2 aria-hidden="true" />
          {t("Open Abyss Value Tables")}
        </Button>
      </CardContent>
    </Card>
  );
}

function SettingsCard({
  icon: Icon,
  titleKey,
  children,
}: {
  icon: typeof Palette;
  titleKey: string;
  children: ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Icon className="size-4" aria-hidden="true" />
          {t(titleKey)}
        </CardTitle>
      </CardHeader>
      <CardContent className="divide-y">{children}</CardContent>
    </Card>
  );
}

function SettingsRow({
  labelKey,
  descriptionKey,
  children,
}: {
  labelKey: string;
  descriptionKey?: string;
  children: ReactNode;
}) {
  return (
    <div className="grid gap-2 py-2.5 first:pt-0 last:pb-0 sm:grid-cols-[minmax(8.5rem,0.42fr)_minmax(0,1fr)] sm:items-center">
      <div>
        <p className="text-sm font-medium">{t(labelKey)}</p>
        {descriptionKey ? (
          <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
            {t(descriptionKey)}
          </p>
        ) : null}
      </div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

function SettingsSwitchRow({
  labelKey,
  checked,
  disabled,
  onCheckedChange,
}: {
  labelKey: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex min-h-9 items-center justify-between gap-4 py-2.5 first:pt-0 last:pb-0">
      <span className="text-sm">{t(labelKey)}</span>
      <Switch
        checked={checked}
        disabled={disabled}
        aria-label={t(labelKey)}
        onCheckedChange={onCheckedChange}
      />
    </label>
  );
}

function LabeledSwitch({
  labelKey,
  checked,
  disabled,
  onCheckedChange,
}: {
  labelKey: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex min-w-0 items-center gap-3">
      <Switch
        checked={checked}
        disabled={disabled}
        aria-label={t(labelKey)}
        onCheckedChange={onCheckedChange}
      />
      <span className="min-w-0 text-sm">{t(labelKey)}</span>
    </label>
  );
}

function NativeSelect({
  className,
  ariaLabel,
  value,
  options,
  disabled,
  onChange,
}: {
  className?: string;
  ariaLabel: string;
  value: string;
  options: ReadonlyArray<{ value: string; label: string }>;
  disabled?: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <select
      className={cn(CONTROL_CLASS_NAME, className)}
      aria-label={ariaLabel}
      value={value}
      disabled={disabled}
      onChange={(event) => onChange(event.target.value)}
    >
      {options.map((option) => (
        <option value={option.value} key={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
}

function submitOnEnter(
  event: ReactKeyboardEvent<HTMLInputElement>,
  submit: () => void,
) {
  if (event.key === "Enter") {
    event.preventDefault();
    submit();
  }
}

function replaceSequential(message: string, values: readonly string[]) {
  return values.reduce(
    (current, value) => current.replace("{}", value),
    message,
  );
}
