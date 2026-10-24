import { Tabs } from "@base-ui/react/tabs";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { settingsFixture } from "../../../dev/fixtures";
import { setFrontendLanguage } from "@/lib/i18n";
import { parseSettingsSnapshot } from "@/lib/tauri/settings-contract";
import {
  SettingsCatalog,
  type SettingsCatalogActions,
} from "./settings-catalog";

const actions: SettingsCatalogActions = {
  setInterface: vi.fn(),
  setUpdatePreferences: vi.fn(),
  checkUpdates: vi.fn(),
  downloadUpdate: vi.fn(),
  installUpdate: vi.fn(),
  setCapture: vi.fn(),
  setMainDpsDisplay: vi.fn(),
  refreshCaptureDevices: vi.fn(),
  applyLayoutProfile: vi.fn(),
  importTeamDataFile: vi.fn(),
  exportTeamData: vi.fn(),
  refreshCaptureFiles: vi.fn(),
  clearCaptureFiles: vi.fn(),
  openAbyssValues: vi.fn(),
};

describe("settings task groups", () => {
  beforeEach(() => setFrontendLanguage("en"));

  it.each(["Interface", "HUD", "Capture and data"] as const)(
    "shows only %s while retaining other drafts",
    (section) => {
      const html = renderToStaticMarkup(
        <Tabs.Root defaultValue={section}>
          <Tabs.List aria-label="Settings">
            {["Interface", "HUD", "Capture and data"].map((label) => (
              <Tabs.Tab key={label} value={label}>
                {label}
              </Tabs.Tab>
            ))}
          </Tabs.List>
          <SettingsCatalog
            snapshot={parseSettingsSnapshot(settingsFixture())}
            pendingAction={null}
            actions={actions}
            hud={<div>HUD controls</div>}
          />
        </Tabs.Root>,
      );
      const panels = [...html.matchAll(/<div([^>]*role="tabpanel"[^>]*)>/g)];
      expect(panels).toHaveLength(3);
      for (const [, attributes] of panels) {
        const label = attributes.match(/aria-label="([^"]+)"/)?.[1];
        expect(attributes.includes('hidden=""')).toBe(label !== section);
      }
      const tabs = [
        ...html.matchAll(/<button([^>]*role="tab"[^>]*)>([^<]+)<\/button>/g),
      ];
      expect(tabs).toHaveLength(3);
      for (const [, attributes, label] of tabs) {
        expect(attributes).toContain(`aria-selected="${label === section}"`);
      }
      for (const label of [
        "Language",
        "Check for updates",
        "HUD controls",
        "Main DPS display",
        "Layout Profiles",
        "Parse Settings",
        "Import DPS Data",
        "Capture Files",
        "Open Abyss Value Tables",
      ]) {
        expect(html).toContain(label);
      }
    },
  );

  it("keeps pending and unavailable actions disabled after regrouping", () => {
    const snapshot = parseSettingsSnapshot(settingsFixture());
    snapshot.teamData.available = false;
    snapshot.captureFiles.count = 0;
    const html = renderToStaticMarkup(
      <Tabs.Root defaultValue="Capture and data">
        <SettingsCatalog
          snapshot={snapshot}
          pendingAction="capture"
          actions={actions}
          hud={null}
        />
      </Tabs.Root>,
    );
    const buttons = [...html.matchAll(/<button([^>]*)>([\s\S]*?)<\/button>/g)];
    for (const label of ["Import DPS Data", "Clear", "Refresh"]) {
      const button = buttons.find(([, , content]) => content.includes(label));
      expect(button?.[1]).toContain('disabled=""');
    }
    expect(html).toContain("Imported team data is unavailable.");
    for (const action of Object.values(actions))
      expect(action).not.toHaveBeenCalled();
  });
});
