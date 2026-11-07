import equipment from "@res/data/equipment/equipment.json";
import { t, useFrontendLanguage } from "@/lib/i18n";
import type { Effect } from "@/lib/tauri/generated/Effect";
import type { EffectModifierOperation } from "@/lib/tauri/generated/EffectModifierOperation";

const attributes = new Map(Object.entries(equipment.attributes));
const numbers = new Intl.NumberFormat("en-US", {
  useGrouping: false,
  maximumSignificantDigits: 7,
});
const operations: Record<EffectModifierOperation, string> = {
  add_base: "Add to base",
  multiply_additive: "Additive multiplier",
  divide_additive: "Additive divisor",
  override: "Override",
  multiply_compound: "Compound multiplier",
  add_final: "Add to final",
};

export function EffectModifierValues({
  modifiers,
}: {
  modifiers: Effect["modifiers"];
}) {
  const language = useFrontendLanguage();
  if (modifiers === null)
    return (
      <p>
        {t(
          "This snapshot did not record evaluated modifiers. Older history cannot be filled from the current game.",
        )}
      </p>
    );
  if (modifiers.status === "unavailable")
    return (
      <p>
        {t(
          "The modifier read failed at capture time. No older values were substituted.",
        )}
      </p>
    );
  const values = modifiers.values;
  if (values.length === 0)
    return (
      <p>{t("No direct attribute modifiers were recorded for this effect.")}</p>
    );
  return (
    <div className="space-y-2">
      <p className="font-medium text-foreground">
        {t("Snapshot modifier values")}
      </p>
      <p>
        {t(
          "Evaluated base magnitudes at this hit, not final stat changes. Stacking, conditions and inhibition are separate.",
        )}
      </p>
      <table className="w-full text-left text-xs">
        <thead>
          <tr>
            {["Attribute", "Operation", "Recorded value"].map((label) => (
              <th key={label} className="py-1 pr-3">
                {t(label)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {values.map((modifier, index) => {
            const attribute = attributes.get(modifier.attribute);
            const translated = attribute
              ? language === "zh-CN"
                ? attribute.name_zh
                : language === "ja"
                  ? attribute.name_ja
                  : attribute.name_en
              : "";
            const label =
              translated || modifier.attribute || t("Unknown attribute");
            const percent =
              attribute?.percent &&
              ["add_base", "add_final", "override"].includes(
                modifier.operation,
              );
            return (
              <tr key={index}>
                <td className="py-1 pr-3">{label}</td>
                <td className="py-1 pr-3">
                  {t(operations[modifier.operation])}
                </td>
                <td className="py-1 font-mono text-foreground">
                  {numbers.format(modifier.magnitude * (percent ? 100 : 1))}
                  {percent ? "%" : ""}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
