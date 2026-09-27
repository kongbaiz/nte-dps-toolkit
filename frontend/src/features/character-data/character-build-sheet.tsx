import { ImageOff } from "lucide-react";
import { t } from "@/lib/i18n";
import type {
  CharacterDetail,
  CharacterEntry,
  CharacterSection,
} from "@/lib/tauri/user-characters-client";
import {
  attributeLabel,
  entryValue,
  equipmentName,
  formatModifier,
  formatPanelNumber,
  isKnownModifier,
  panelImage,
} from "./character-build-model";
import "./character-build.css";

type Field = CharacterEntry["fields"][number];
const METADATA = new Set([
  "Item ID",
  "Slot state",
  "Slot source",
  "Equipment UID",
  "Equipment details",
  "Attribute stage",
  "Attribute units",
  "Base modifier ID",
]);
function Artwork({
  kind,
  id,
  name,
}: {
  kind: "character" | "arc" | "equipment";
  id: string | null;
  name: string;
}) {
  const src = panelImage(kind, id);
  return src ? (
    <img
      className="build-art"
      src={src}
      alt={name}
      loading="lazy"
      draggable={false}
    />
  ) : (
    <div
      className="build-art-missing"
      role="img"
      aria-label={t("Artwork unavailable")}
    >
      <ImageOff aria-hidden="true" />
      <span>{t("Artwork unavailable")}</span>
    </div>
  );
}
function FieldRows({
  fields,
  modifier = false,
}: {
  fields: Field[];
  modifier?: boolean;
}) {
  return (
    <dl className="build-stat-rows">
      {fields.map((field, i) => (
        <div key={`${field.label}:${i}`}>
          <dt>{modifier ? attributeLabel(field.label) : t(field.label)}</dt>
          <dd title={`${t("Raw value")}: ${field.value ?? t("Unknown value")}`}>
            {modifier
              ? formatModifier(field.label, field.value)
              : (field.value ?? "—")}
          </dd>
        </div>
      ))}
    </dl>
  );
}
function SectionNotice({ section }: { section: CharacterSection | undefined }) {
  return !section || !section.available ? (
    <p className="build-muted">{t("Not fully observed")}</p>
  ) : section.entries.length === 0 ? (
    <p className="build-muted">{t("None observed")}</p>
  ) : null;
}
function EquipmentCard({
  entry,
  kind,
  category,
}: {
  entry: CharacterEntry;
  kind: "arc" | "equipment";
  category?: "Cassettes" | "Drive blocks";
}) {
  const id = entryValue(entry, "Item ID");
  const name = entry.name || equipmentName(id) || id || t("Unknown value");
  const level = entryValue(entry, "Enhancement level");
  const fields = entry.fields.filter(
    (f) => !METADATA.has(f.label) && f.label !== "Enhancement level",
  );
  const metadata = entry.fields.filter((f) => METADATA.has(f.label));
  return (
    <article
      className={`build-equipment-card ${kind === "arc" ? "build-arc-card" : ""}`}
    >
      <header>
        <div className="build-equipment-image">
          <Artwork kind={kind} id={id} name={name} />
          <span>Lv.{formatPanelNumber(level, "integer")}</span>
        </div>
        <div className="build-equipment-heading">
          <h4 title={name}>{name}</h4>
          {kind === "arc" ? (
            <p>
              {t("Star level")}{" "}
              {formatPanelNumber(entryValue(entry, "Star level"), "integer")} ·{" "}
              {t("Breakthrough level")}{" "}
              {formatPanelNumber(
                entryValue(entry, "Breakthrough level"),
                "integer",
              )}
            </p>
          ) : (
            <p>{t(category ?? "Equipment snapshot")}</p>
          )}
        </div>
      </header>
      {kind === "equipment" ? <FieldRows fields={fields} modifier /> : null}
      <details className="build-source">
        <summary>{t("Source details")}</summary>
        <FieldRows fields={metadata} />
        {kind === "equipment" &&
        fields.some((f) => !isKnownModifier(f.label)) ? (
          <p>{t("Unknown modifiers retain raw units.")}</p>
        ) : null}
      </details>
    </article>
  );
}
const RUNTIME_ROWS = [
  ["HPMaxBase", "HP"],
  ["AtkBase", "Attack"],
  ["DefBase", "Defense"],
  ["CritBase", "Critical rate"],
  ["CritDamageBase", "Critical damage"],
  ["DamageUpGeneralBase", "Universal DMG Bonus"],
  ["ChargeGetEfficiencyBase", "Charge efficiency"],
] as const;
export function CharacterSnapshotDetail({
  detail,
}: {
  detail: CharacterDetail;
}) {
  const sections = new Map(detail.sections.map((s) => [s.title, s]));
  const progression = sections.get("Character progression"),
    skills = sections.get("Skill levels"),
    awaken = sections.get("Active awakenings"),
    arc = sections.get("Arc"),
    cassettes = sections.get("Cassettes"),
    blocks = sections.get("Drive blocks");
  const runtime = sections.get("Runtime attributes");
  const runtimeFields = runtime?.entries[0]?.fields;
  const c = detail.summary;
  const name = c.name || c.itemId || t("Unknown character");
  const awakening = progression?.entries[0]
    ? entryValue(progression.entries[0], "Awakening level")
    : null;
  return (
    <div className="character-build-sheet">
      <section className="build-hero" aria-label={t("Character overview")}>
        <div className="build-portrait">
          <Artwork kind="character" id={c.itemId} name={name} />
          <div className="build-portrait-footer">
            <span className="build-awakening">
              {t("Awakening level")} {formatPanelNumber(awakening, "integer")}
            </span>
            <section aria-label={t("Skill levels")} className="build-skills">
              <SectionNotice section={skills} />
              {skills?.entries.map((entry, i) => (
                <div
                  key={i}
                  title={entry.fields
                    .map(
                      (f) => `${t(f.label)}: ${f.value ?? t("Unknown value")}`,
                    )
                    .join("\n")}
                >
                  <span className="build-skill-level">
                    {formatPanelNumber(
                      entryValue(entry, "Effective level"),
                      "integer",
                    )}
                  </span>
                  <span>{entry.name ? t(entry.name) : t("Unknown skill")}</span>
                </div>
              ))}
            </section>
          </div>
        </div>
        <div className="build-character-info">
          <header>
            <div>
              <h2>{name}</h2>
              <span className="build-level">
                Lv.{formatPanelNumber(c.level, "integer")}
              </span>
            </div>
          </header>
          <section className="build-attributes">
            <h3>{t("Runtime attributes")}</h3>
            <dl className="build-runtime-stats">
              {RUNTIME_ROWS.map(([key], index) => (
                <div
                  key={key}
                  className={
                    index < 3 ? "build-primary-stat" : "build-secondary-stat"
                  }
                >
                  <dt>{attributeLabel(key)}</dt>
                  <dd
                    title={
                      runtimeFields?.find((f) => f.label === key)?.value ??
                      t("Unknown value")
                    }
                  >
                    {formatPanelNumber(
                      runtimeFields?.find((f) => f.label === key)?.value ??
                        null,
                      key === "HPMaxBase" || key === "DefBase"
                        ? "integer"
                        : key === "AtkBase"
                          ? "attack"
                          : "percent",
                    )}
                  </dd>
                </div>
              ))}
            </dl>
            <p className="build-attribute-note">
              {t(
                runtime?.available
                  ? runtime.entries[0]?.name ===
                    "Account configuration attributes"
                    ? "Account attributes computed by the game from current character data."
                    : "Live team attributes sampled at refresh; current buffs are included."
                  : "Live attributes unavailable; the account attribute calculation could not be verified.",
              )}
            </p>
          </section>
        </div>
      </section>
      <div className="build-middle">
        <section aria-label={t("Arc")}>
          <h3 className="build-section-title">{t("Arc")}</h3>
          <SectionNotice section={arc} />
          {arc?.entries.map((entry, i) => (
            <EquipmentCard entry={entry} kind="arc" key={i} />
          ))}
        </section>
        <section className="build-awakenings">
          <h3>{t("Active awakenings")}</h3>
          <SectionNotice section={awaken} />
          <div>
            {awaken?.entries.map((entry, i) => (
              <span
                className="build-awakening-chip"
                key={i}
                title={entryValue(entry, "Effect ID") ?? undefined}
              >
                <b>
                  {formatPanelNumber(
                    entryValue(entry, "Awakening number"),
                    "integer",
                  )}
                </b>
                {entryValue(entry, "Effect name") ||
                  entryValue(entry, "Effect ID") ||
                  t("Unknown value")}
              </span>
            ))}
          </div>
        </section>
      </div>
      <section
        className="build-equipment"
        aria-label={t("Cassettes and drive blocks")}
      >
        <div className="build-section-header">
          <h3>
            {t("Cassettes")} / {t("Drive blocks")}
          </h3>
          <span>{t("Equipment modifiers")}</span>
        </div>
        <p className="build-unit-note">
          {t(
            "Known modifier units follow the equipment catalog; unknown modifiers retain raw units.",
          )}
        </p>
        <SectionNotice section={cassettes} />
        <SectionNotice section={blocks} />
        <div className="build-equipment-grid">
          {cassettes?.entries.map((entry, i) => (
            <EquipmentCard
              entry={entry}
              kind="equipment"
              category="Cassettes"
              key={`cassette:${i}`}
            />
          ))}
          {blocks?.entries.map((entry, i) => (
            <EquipmentCard
              entry={entry}
              kind="equipment"
              category="Drive blocks"
              key={`drive:${i}`}
            />
          ))}
        </div>
      </section>
      <details className="build-source build-extra">
        <summary>{t("Progression and skill details")}</summary>
        <p className="build-character-uid">
          {t("Character UID")}: {c.uid}
        </p>
        <h3>{t("Character progression")}</h3>
        {progression?.entries.map((entry, i) => (
          <FieldRows fields={entry.fields} key={i} />
        ))}
        <h3>{t("Skill levels")}</h3>
        {skills?.entries.map((entry, i) => (
          <FieldRows fields={entry.fields} key={i} />
        ))}
      </details>
    </div>
  );
}
