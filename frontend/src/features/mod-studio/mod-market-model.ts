import type { SettingsLanguage } from "@/lib/tauri/settings-contract";
import type {
  ModMarketItem,
  ModMarketLocalizedText,
} from "@/lib/tauri/mod-studio-contract";

export function localizedModMarketText(
  item: ModMarketItem,
  language: SettingsLanguage,
): ModMarketLocalizedText {
  return item.localizations[language];
}

export function modMarketSearchText(
  item: ModMarketItem,
  language: SettingsLanguage,
): string {
  const localized = localizedModMarketText(item, language);
  return [
    item.id,
    localized.name,
    localized.summary,
    item.author,
    ...item.capabilities,
    ...item.bindings,
  ].join("\n");
}
