import { describe, expect, it } from "vitest";

import consolePageSource from "../../features/console/console-page.tsx?raw";
import encryptedIniSource from "../../features/encrypted-ini/encrypted-ini-page.tsx?raw";
import historyPageSource from "../../features/history/history-page.tsx?raw";
import updatePromptSource from "../../features/main-dps/update-prompt-dialog.tsx?raw";
import mainDpsDetailSource from "../../features/main-dps/main-dps-detail-page.tsx?raw";
import mainDpsSource from "../../features/main-dps/main-dps-page.tsx?raw";
import modStudioSource from "../../features/mod-studio/mod-studio-page.tsx?raw";
import technicalHudSource from "../../features/technical-hud/technical-hud-page.tsx?raw";
import timelineChartSource from "../../features/timeline/timeline-chart.tsx?raw";
import indexCssSource from "../../index.css?raw";

const uiPrimitiveSources = import.meta.glob("./{alert-dialog,menu}.tsx", {
  eager: true,
  import: "default",
  query: "?raw",
}) as Record<string, string>;
const legacyDismissibleLayerSources = import.meta.glob(
  "../../hooks/use-dismissible-layer.ts",
  {
    eager: true,
    import: "default",
    query: "?raw",
  },
) as Record<string, string>;

const migratedProductionSources = [
  consolePageSource,
  encryptedIniSource,
  historyPageSource,
  mainDpsSource,
  modStudioSource,
  technicalHudSource,
  timelineChartSource,
  indexCssSource,
].join("\n");

describe("Base UI migration invariants", () => {
  it("keeps the update dialog popup above its high-priority backdrop", () => {
    expect(updatePromptSource).toContain(
      '<DialogBackdrop className="z-[90]" />',
    );
    expect(updatePromptSource).toMatch(
      /<DialogPopup className="[^"]*z-\[91\][^"]*"/,
    );
  });

  it("uses the Base UI close primitive for the update dialog", () => {
    expect(updatePromptSource).toContain("<DialogClose");
    expect(updatePromptSource).toContain("render={<Button");
  });

  it("closes the column popover when the native window loses focus", () => {
    expect(mainDpsDetailSource).toContain(
      'window.addEventListener("blur", closeColumnsOnWindowBlur)',
    );
    expect(mainDpsDetailSource).toContain(
      'window.removeEventListener("blur", closeColumnsOnWindowBlur)',
    );
  });

  it("provides project AlertDialog and Menu primitives backed by Base UI", () => {
    const alertDialogSource = uiPrimitiveSources["./alert-dialog.tsx"];
    const menuSource = uiPrimitiveSources["./menu.tsx"];

    expect(alertDialogSource).toContain("@base-ui/react/alert-dialog");
    expect(menuSource).toContain("@base-ui/react/menu");
    expect(menuSource).toContain("@base-ui/react/context-menu");
    expect(menuSource).toContain("MenuPrimitive.Item");
  });

  it("removes the manual dismiss hook and raw dialog or menu roles", () => {
    expect(Object.keys(legacyDismissibleLayerSources)).toHaveLength(0);
    expect(migratedProductionSources).not.toMatch(
      /useDismissibleLayer|role=["'](?:dialog|menu)["']/,
    );
  });

  it("migrates every reported W-03 consumer to project primitives", () => {
    expect(consolePageSource).toContain("<AlertDialog");
    expect(encryptedIniSource).toContain("<AlertDialog");
    expect(modStudioSource.match(/<AlertDialog(?:\s|>)/g)).toHaveLength(2);
    expect(mainDpsSource).toContain("<AlertDialog");
    expect(mainDpsSource).toContain("<Dialog");
    expect(mainDpsSource).toContain("<Popover");
    expect(mainDpsSource).toContain("<ContextMenu");
    expect(historyPageSource).toContain("<ContextMenu");
    expect(timelineChartSource).toContain("<ContextMenu");
    expect(technicalHudSource).toContain("<Popover");
  });

  it("keeps native-window blur dismissal for migrated popovers", () => {
    expect(mainDpsSource).toContain('window.addEventListener("blur"');
    expect(technicalHudSource).toContain('window.addEventListener("blur"');
  });
});
