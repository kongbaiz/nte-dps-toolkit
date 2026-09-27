import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import { setFrontendLanguage } from "@/lib/i18n";
import type { PluginPanel } from "@/lib/tauri/toolkit-client";
import { RELEASE_ACTIONS } from "@/lib/tauri/release-plugin-client";
import { HostLoadingControls } from "./host-loading-controls";
import { HostControls } from "./host-controls";
import { PluginControls } from "./plugin-controls";
import { pluginControlGroup } from "./release-control-groups";

const panel: PluginPanel = {
  contractVersion: 4,
  loadingMethod: "proxy",
  connectionIdentity: "123:456",
  collectorActive: false,
  mode: "plugin",
  connection: "connected",
  componentsReady: true,
  capabilities: [4, 5, 6, 8, ...RELEASE_ACTIONS.map((action) => action[1])],
  plugins: ["Combat", "User", "Network", "Performance"].map((name) => ({
    file: `NTE_Plugin${name}.dll`,
    state: "loaded",
  })),
  combat: null,
  operation: { state: "", error: "" },
};
const run = async () => {};
const common = {
  panel,
  pending: false,
  run,
  refresh: async () => {},
  onOpenMarket: () => {},
};
const renderPlugin = (file: string, snapshot = panel) =>
  renderToStaticMarkup(
    <PluginControls
      {...common}
      panel={snapshot}
      plugin={snapshot.plugins.find((p) => p.file === file)!}
      title={file}
    />,
  );

describe("Mod Workshop owner pages", () => {
  beforeEach(() => setFrontendLanguage("en"));
  it("offers ordered setup guidance and a direct market entry", () => {
    const html = renderToStaticMarkup(<HostControls {...common} />);
    expect(html).toContain('aria-label="Getting started"');
    expect(html).toContain("Open Mod Market");
    const steps = html.match(/<ol\b[^>]*>[\s\S]*?<\/ol>/)![0];
    expect(steps.indexOf("Install components")).toBeLessThan(
      steps.indexOf("Load host"),
    );
    expect(steps.indexOf("Load host")).toBeLessThan(
      steps.indexOf("Use plugins"),
    );
    expect(html).toContain(
      "Select a plugin above to load it or adjust its features.",
    );
  });
  it.each([
    [
      { mode: "packet_capture" },
      "Choose Plugin mode in the home page data source selector first.",
      true,
    ],
    [
      { connection: "unavailable", componentsReady: false },
      "Install the required components from the Mod Market first.",
      true,
    ],
    [
      { collectorActive: true },
      "Stop collection before changing plugin lifecycle state.",
      true,
    ],
    [
      { connection: "unavailable", loadingMethod: "proxy" },
      "Close the game, load the host to deploy the proxy, then restart the game and check status.",
      false,
    ],
    [
      { connection: "unavailable", loadingMethod: "loader" },
      "Start the game, then load the host. Approve UAC if prompted.",
      false,
    ],
    [{}, "Select a plugin above to load it or adjust its features.", true],
  ] satisfies [Partial<PluginPanel>, string, boolean][])(
    "explains the next action for %j",
    (overrides, guidance, disabled) => {
      const html = renderToStaticMarkup(
        <HostControls {...common} panel={{ ...panel, ...overrides }} />,
      );
      expect(html).toContain(guidance);
      const load = html
        .match(/<button\b[^>]*>[\s\S]*?<\/button>/g)!
        .find((button) => button.includes("Load host"))!;
      expect(load).toContain('aria-describedby="host-load-guidance"');
      expect(load.includes('disabled=""')).toBe(disabled);
    },
  );
  it("keeps method-specific instructions visible and secondary notes collapsed", () => {
    const html = renderToStaticMarkup(<HostLoadingControls {...common} />);
    expect(html.indexOf("Restart the game after deployment.")).toBeLessThan(
      html.indexOf("<details"),
    );
    expect(html).not.toMatch(/<details[^>]*\bopen/);
    expect(html).toContain(
      "Changing this preference does not unload the running host",
    );
  });
  it("limits the host to status, load and unload without manual configuration", () => {
    const html = renderToStaticMarkup(
      <HostControls {...common} refresh={async () => {}} />,
    );
    for (const label of [
      "Host loading method",
      "Proxy loading",
      "Loader loading",
      "Check status",
      "Load host",
      "Host status",
      "Unload host",
    ])
      expect(html).toContain(label);
    for (const label of [
      "Toolkit directory",
      "Host logging",
      "SDK compatibility",
      "Retry SDK scan",
      "In-game enhanced HUD",
      "Preview native combat report",
      "Refresh account data",
      "Plugin network recording",
      "Refresh plugin runtime snapshot",
    ])
      expect(html).not.toContain(label);
  });
  for (const file of [
    "NTE_PluginCombat.dll",
    "NTE_PluginUser.dll",
    "NTE_PluginNetwork.dll",
  ]) {
    it(`renders only actions owned by ${file}`, () => {
      const html = renderPlugin(file);
      const group = pluginControlGroup(file)!;
      const own: readonly string[] = [...group.actions, ...group.advanced];
      for (const [action, , label] of RELEASE_ACTIONS) {
        if (["hudStatus", "hudConfigure", "combatStopExport"].includes(action))
          continue;
        if (own.includes(action)) expect(html).toContain(label);
        else expect(html).not.toContain(label);
      }
      expect(html).not.toContain("Host logging");
      expect(html).not.toContain("Toolkit directory");
      expect(html).not.toContain('role="tablist"');
      if (group.id === "combat") expect(html).toContain("In-game enhanced HUD");
      else expect(html).not.toContain("In-game enhanced HUD");
    });
  }
  it("gives unknown plugins their own lifecycle page without guessing capabilities", () => {
    const html = renderPlugin("NTE_PluginPerformance.dll");
    expect(html).toContain("NTE_PluginPerformance.dll");
    expect(html).toContain("Check status");
    expect(html).toContain("Load plugin");
    expect(html).toContain("Unload plugin");
    expect(html).not.toContain("Reload");
    expect(html).toContain("no feature controls exposed");
    expect(html).not.toContain("In-game enhanced HUD");
    expect(html).not.toContain("Preview native combat report");
  });
  it("disables lifecycle controls while unloading or when host capabilities are absent", () => {
    for (const snapshot of [
      { ...panel, capabilities: [] },
      {
        ...panel,
        plugins: panel.plugins.map((plugin) =>
          plugin.file === "NTE_PluginPerformance.dll"
            ? { ...plugin, state: "unload_pending" as const }
            : plugin,
        ),
      },
    ]) {
      const html = renderPlugin("NTE_PluginPerformance.dll", snapshot);
      const buttons = html.match(/<button\b[^>]*>[\s\S]*?<\/button>/g)!;
      expect(buttons.length).toBe(3);
      for (const button of buttons.filter(
        (button) => !button.includes("Check status"),
      ))
        expect(button).toContain('disabled=""');
    }
  });
  it("does not enable lifecycle mutations during collection", () => {
    const snapshot = { ...panel, collectorActive: true };
    const html = renderPlugin("NTE_PluginPerformance.dll", snapshot);
    const buttons = html.match(/<button\b[^>]*>[\s\S]*?<\/button>/g)!;
    expect(buttons.length).toBe(3);
    for (const button of buttons.filter(
      (button) => !button.includes("Check status"),
    ))
      expect(button).toContain('disabled=""');
    expect(html).toContain(
      "Stop collection before changing plugin lifecycle state.",
    );
  });
});

it("keeps the loading preference reusable on the host panel without asking for a path", () => {
  setFrontendLanguage("en");
  const html = renderToStaticMarkup(<HostLoadingControls {...common} />);
  expect(html).toContain("Proxy loading");
  expect(html).toContain("Loader loading");
  expect(html).toContain("managed mods directory");
  expect(html).not.toContain("Select directory");
  expect(html).not.toContain("Start or connect with Loader");
});
