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

export interface ModMarketLocalStatus {
  installed: boolean;
  enabled: boolean;
  current: boolean;
  unreadable: { code: string; messageKey: string } | null;
}

export function modMarketLocalStatus(
  item: ModMarketItem,
): ModMarketLocalStatus {
  switch (item.localState.status) {
    case "notInstalled":
      return {
        installed: false,
        enabled: false,
        current: false,
        unreadable: null,
      };
    case "installed":
      return {
        installed: true,
        enabled: item.localState.enabled,
        current: item.localState.current,
        unreadable: null,
      };
    case "unreadable":
      return {
        installed: false,
        enabled: false,
        current: false,
        unreadable: {
          code: item.localState.code,
          messageKey: item.localState.messageKey,
        },
      };
  }
}
