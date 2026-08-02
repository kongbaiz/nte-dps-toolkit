import { afterEach, describe, expect, it } from "vitest";

import { setFrontendLanguage, t } from "./i18n";

const MIGRATION_PARITY_KEYS = [
  "Boss",
  "Failed to import team data.",
  "Team data file dialog failed.",
  "Toggle mouse passthrough while the combat HUD is active",
  "DPS: {}",
  "Share: {}%",
  "Taken: {}",
] as const;

afterEach(() => setFrontendLanguage("zh-CN"));

describe("frontend i18n parity", () => {
  it.each(["zh-CN", "ja"] as const)(
    "localizes the remaining migrated UI keys in %s",
    (language) => {
      setFrontendLanguage(language);
      for (const key of MIGRATION_PARITY_KEYS) {
        expect(t(key)).not.toBe(key);
      }
    },
  );
});
