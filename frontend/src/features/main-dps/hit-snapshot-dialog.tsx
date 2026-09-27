import { useEffect, useState } from "react";
import { X, RefreshCw } from "lucide-react";
import {
  Dialog,
  DialogPortal,
  DialogBackdrop,
  DialogPopup,
  DialogTitle,
  DialogDescription,
  DialogClose,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { t } from "@/lib/i18n";
import { mainDpsDetailClient } from "@/lib/tauri/main-dps-detail-client";
import type { MainDpsHit } from "@/lib/tauri/main-dps-detail-contract";
import type {
  HitSnapshot,
  AttributeSnapshot,
  EffectSnapshot,
} from "@/lib/tauri/hit-snapshot-contract";
import { criticalTranslationKey } from "./main-dps-detail-model";

const ATTRIBUTE_LABELS: Record<string, string> = {
  hp: "HP",
  maxHp: "Max HP",
  shield: "Shield",
  attack: "Attack",
  defense: "Defense",
  crit: "Critical rate",
  critDamage: "Critical damage",
  resistNormal: "Normal resistance",
  resistCosmos: "Cosmos resistance",
  resistNature: "Nature resistance",
  resistIncantation: "Incantation resistance",
  resistChaos: "Chaos resistance",
  resistPsyche: "Psyche resistance",
  resistLakshana: "Lakshana resistance",
  resistPsychically: "Psychic resistance",
  damageUpGeneral: "General damage bonus",
  damageUpNormal: "Normal damage bonus",
  damageUpCosmos: "Cosmos damage bonus",
  damageUpNature: "Nature damage bonus",
  damageUpIncantation: "Incantation damage bonus",
  damageUpChaos: "Chaos damage bonus",
  damageUpPsyche: "Psyche damage bonus",
  damageUpLakshana: "Lakshana damage bonus",
  damageUpPsychically: "Psychic damage bonus",
  chargeCurrent: "Current charge",
  chargeMax: "Maximum charge",
  unbalCurrent: "Current unbalance",
  unbalMax: "Maximum unbalance",
  unbalAccrueEfficiency: "Unbalance accumulation efficiency",
  unbalAntiAccrueEfficiency: "Unbalance accumulation resistance",
  unbalSpeed: "Unbalance speed",
  unbalIntensity: "Unbalance intensity",
  unbalBonus: "Unbalance bonus",
  unbalReduceNatur: "Natural unbalance reduction",
  unbalValueAdd: "Additional unbalance",
  isBalancedingState: "IsBalancedingState",
};
// Match the plugin's explicit AttributeIsPercentage list; never infer units
// from the magnitude or name of an unknown future attribute.
const PERCENTAGE_ATTRIBUTES = new Set([
  "crit",
  "critDamage",
  "resistNormal",
  "resistCosmos",
  "resistNature",
  "resistIncantation",
  "resistChaos",
  "resistPsyche",
  "resistLakshana",
  "resistPsychically",
  "damageUpGeneral",
  "damageUpNormal",
  "damageUpCosmos",
  "damageUpNature",
  "damageUpIncantation",
  "damageUpChaos",
  "damageUpPsyche",
  "damageUpLakshana",
  "damageUpPsychically",
  "unbalAccrueEfficiency",
  "unbalAntiAccrueEfficiency",
  "unbalBonus",
]);
const numberFormat = new Intl.NumberFormat("en-US", {
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
  useGrouping: false,
});
const percentFormat = new Intl.NumberFormat("en-US", {
  style: "percent",
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
  useGrouping: false,
});
function attributeValue(key: string, value: number | boolean | null) {
  if (value === null) return t("Unknown value");
  if (typeof value === "boolean") return t(value ? "Yes" : "No");
  if (PERCENTAGE_ATTRIBUTES.has(key)) return percentFormat.format(value);
  return Object.hasOwn(ATTRIBUTE_LABELS, key)
    ? numberFormat.format(value)
    : String(value);
}
export function AttributeSnapshotView({
  snapshot,
}: {
  snapshot: AttributeSnapshot | null;
}) {
  if (!snapshot?.values)
    return (
      <p className="p-5 text-sm text-muted-foreground">
        {t("No attributes were recorded for this hit.")}
      </p>
    );
  return (
    <div className="space-y-3">
      <p className="break-words text-xs text-muted-foreground">
        {snapshot.actorName} · {t("Snapshot ID")}:{" "}
        {snapshot.id ?? t("Unknown value")} ·{" "}
        {snapshot.status ?? t("Unknown value")} · {t("Sample time (µs)")}:{" "}
        {snapshot.sampledUnixUs ?? t("Unknown value")}
      </p>
      <p className="text-xs text-muted-foreground">
        {t(
          "Values and percentages use two decimal places; original snapshot precision is preserved.",
        )}
      </p>
      <div className="overflow-x-auto rounded-lg border">
        <table className="w-full text-left text-sm">
          <thead className="bg-muted/50 text-xs">
            <tr>
              <th className="p-3">{t("Attribute")}</th>
              <th className="p-3">{t("Value")}</th>
              <th className="p-3">{t("Status / source")}</th>
            </tr>
          </thead>
          <tbody>
            {Object.entries(snapshot.values).map(([key, entry]) => (
              <tr key={key} className="border-t align-top">
                <th className="px-3 py-2.5 font-normal">
                  {Object.hasOwn(ATTRIBUTE_LABELS, key)
                    ? t(ATTRIBUTE_LABELS[key])
                    : key}
                </th>
                <td className="px-3 py-2.5 font-mono tabular-nums">
                  {attributeValue(key, entry.value)}
                </td>
                <td className="max-w-72 break-all px-3 py-2.5 text-xs text-muted-foreground">
                  {entry.status}
                  <br />
                  {entry.source}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
export function EffectSnapshotView({
  snapshot,
}: {
  snapshot: EffectSnapshot | null;
}) {
  const [limit, setLimit] = useState(50);
  if (!snapshot?.effects)
    return (
      <p className="p-5 text-sm text-muted-foreground">
        {t("No effects were recorded for this hit.")}
      </p>
    );
  return (
    <div className="space-y-3">
      <p className="text-xs text-muted-foreground">
        {t("Snapshot ID")}: {snapshot.id ?? t("Unknown value")} ·{" "}
        {snapshot.status ?? t("Unknown value")} · {t("Sample time (µs)")}:{" "}
        {snapshot.observedUs ?? t("Unknown value")}
      </p>
      {snapshot.complete === false && (
        <p role="status" className="text-sm text-amber-600">
          {t("Partial snapshot: only the observed entries are listed.")}
        </p>
      )}
      {snapshot.complete === null && (
        <p role="status" className="text-sm text-amber-600">
          {t("Snapshot completeness is unknown.")}
        </p>
      )}
      <p className="text-xs text-muted-foreground">
        {t(
          "Effects present at this hit; this does not mean the hit applied these effects.",
        )}
      </p>
      {snapshot.effects.length === 0 ? (
        <p className="p-5 text-sm">
          {t(
            snapshot.complete === true
              ? "No effects"
              : "No observed effect entries.",
          )}
        </p>
      ) : (
        <div className="overflow-x-auto rounded-lg border">
          <table className="w-full text-left text-sm">
            <thead className="bg-muted/50 text-xs">
              <tr>
                {[
                  "Effect",
                  "Type",
                  "Stacks",
                  "Duration (seconds)",
                  "Source",
                ].map((k) => (
                  <th key={k} className="p-3">
                    {t(k)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {snapshot.effects.slice(0, limit).map((e, index) => (
                <tr
                  key={`${e.instanceKey}:${index}`}
                  className="border-t align-top"
                >
                  <td className="max-w-96 px-3 py-3">
                    <details>
                      <summary className="cursor-pointer break-words font-medium">
                        {e.name || e.key || t("Unnamed effect")}
                      </summary>
                      <div className="mt-2 space-y-2 whitespace-pre-wrap break-words text-xs text-muted-foreground">
                        <p>{e.description}</p>
                        <p>{e.key}</p>
                        <p>
                          {t("Snapshot ID")}: {e.instanceKey}
                        </p>
                        <p>
                          {t("Start world time")}:{" "}
                          {numberFormat.format(e.startWorldTime)} · {t("Level")}
                          : {e.level}
                        </p>
                      </div>
                    </details>
                    {e.inhibited && (
                      <span className="text-xs text-amber-600">
                        {t("Inhibited")}
                      </span>
                    )}
                  </td>
                  <td className="px-3 py-3">
                    {t(
                      e.kind === 2
                        ? "Positive effect"
                        : e.kind === 3
                          ? "Negative effect"
                          : e.kind === 1
                            ? "State effect"
                            : "Other",
                    )}
                  </td>
                  <td className="px-3 py-3 font-mono">{e.stacks}</td>
                  <td className="px-3 py-3 font-mono">
                    {e.duration < 0
                      ? t("Permanent")
                      : numberFormat.format(e.duration)}
                  </td>
                  <td className="max-w-44 break-all px-3 py-3 text-xs text-muted-foreground">
                    {e.source}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>
          {Math.min(limit, snapshot.effects.length)} / {snapshot.effects.length}
        </span>
        {limit < snapshot.effects.length && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => setLimit((v) => v + 50)}
          >
            {t("Load more")}
          </Button>
        )}
      </div>
    </div>
  );
}
export function HitSnapshotDialog({
  row,
  onClose,
}: {
  row: MainDpsHit;
  onClose: () => void;
}) {
  const [snapshot, setSnapshot] = useState<HitSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [section, setSection] = useState("roleAttributes");
  useEffect(() => {
    let active = true;
    if (row.snapshotKey !== null)
      void mainDpsDetailClient
        .getHitSnapshot(row.id, row.snapshotKey)
        .then((next) => {
          if (active) setSnapshot(next);
        })
        .catch((e) => {
          if (active)
            setError(
              typeof e === "object" &&
                e !== null &&
                "messageKey" in e &&
                typeof e.messageKey === "string"
                ? e.messageKey
                : "Failed to load this hit snapshot.",
            );
        });
    return () => {
      active = false;
    };
  }, [row.id, row.snapshotKey]);
  const incoming = row.direction === "incoming";
  const roleAttributes = snapshot?.data
    ? incoming
      ? snapshot.data.victimAttributes
      : snapshot.data.attackerAttributes
    : null;
  const enemyAttributes = snapshot?.data
    ? incoming
      ? snapshot.data.attackerAttributes
      : snapshot.data.victimAttributes
    : null;
  const roleEffects = snapshot?.data
    ? incoming
      ? snapshot.data.victimEffects
      : snapshot.data.attackerEffects
    : null;
  const enemyEffects = snapshot?.data
    ? incoming
      ? snapshot.data.attackerEffects
      : snapshot.data.victimEffects
    : null;
  const sections = [
    [
      "roleAttributes",
      row.direction === "unknown" ? "Attacker attributes" : "Role attributes",
    ],
    [
      "enemyAttributes",
      row.direction === "unknown" ? "Victim attributes" : "Enemy attributes",
    ],
    [
      "roleEffects",
      row.direction === "unknown" ? "Attacker effects" : "Role effects",
    ],
    [
      "enemyEffects",
      row.direction === "unknown" ? "Victim effects" : "Enemy effects",
    ],
  ];
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogPortal>
        <DialogBackdrop />
        <DialogPopup className="left-1/2 top-1/2 flex max-h-[90vh] w-[min(1100px,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl border bg-background shadow-xl">
          <header className="flex items-start justify-between gap-4 border-b p-5">
            <div>
              <DialogTitle>{t("Hit snapshot details")}</DialogTitle>
              <DialogDescription>
                {t(
                  "Frozen observations for this hit. Missing values are never filled from live state.",
                )}
              </DialogDescription>
              <p className="mt-3 text-sm">
                {row.characterName} · {row.typeLabel} ·{" "}
                {new Date(row.timestamp * 1000).toLocaleTimeString()} ·{" "}
                {row.damage.toLocaleString()} · {t("Critical hit")}:{" "}
                {t(criticalTranslationKey(row.critical))}
              </p>
              {snapshot?.criticalSource && (
                <p className="mt-1 text-xs text-muted-foreground">
                  {t("Critical evidence")}: {snapshot.criticalSource}
                </p>
              )}
            </div>
            <DialogClose
              className="rounded-md p-1 hover:bg-muted"
              aria-label={t("Close")}
            >
              <X className="size-5" />
            </DialogClose>
          </header>
          {error ? (
            <p role="alert" className="p-5 text-destructive">
              {t(error)}
            </p>
          ) : snapshot === null ? (
            <p className="flex items-center gap-2 p-5">
              <RefreshCw className="size-4 animate-spin" />
              {t("Loading...")}
            </p>
          ) : snapshot.data === null ? (
            <p className="p-5 text-sm text-muted-foreground">
              {t(
                snapshot.retention === "budget_exceeded"
                  ? "Snapshot detail was not retained because the capture budget was reached."
                  : "No snapshot detail was recorded for this hit.",
              )}
            </p>
          ) : (
            <>
              <nav
                aria-label={t("Hit snapshot details")}
                className="flex shrink-0 gap-1 overflow-x-auto border-b px-4 py-2"
              >
                {sections.map(([id, label]) => (
                  <Button
                    key={id}
                    size="sm"
                    variant={section === id ? "secondary" : "ghost"}
                    aria-pressed={section === id}
                    onClick={() => setSection(id)}
                  >
                    {t(label)}
                  </Button>
                ))}
              </nav>
              <div className="min-h-0 overflow-y-auto p-5">
                {section === "roleAttributes" ? (
                  <AttributeSnapshotView snapshot={roleAttributes} />
                ) : section === "enemyAttributes" ? (
                  <AttributeSnapshotView snapshot={enemyAttributes} />
                ) : (
                  <EffectSnapshotView
                    key={section}
                    snapshot={
                      section === "roleEffects" ? roleEffects : enemyEffects
                    }
                  />
                )}
              </div>
            </>
          )}
        </DialogPopup>
      </DialogPortal>
    </Dialog>
  );
}
