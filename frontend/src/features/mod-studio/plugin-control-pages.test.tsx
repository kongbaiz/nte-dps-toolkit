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
  contractVersion: 5,
  loadingMethod: "proxy",
  gameDirectory: null,
  connectionIdentity: "123:456",
  collectorActive: false,
  mode: "plugin",
  connection: "connected",
  componentsReady: true,
  capabilities: [
    ...new Set([4, 5, 6, 8, ...RELEASE_ACTIONS.map((action) => action[1])]),
  ],
  plugins: ["Combat", "User", "Network", "Training", "Custom"].map((name) => ({
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
  it("shows setup only when components are missing and collapses connected settings", () => {
    const connected = renderToStaticMarkup(<HostControls {...common} />);
    expect(connected).not.toContain('aria-label="Getting started"');
    expect(connected).not.toContain('aria-describedby="host-load-guidance"');
    expect(connected).not.toMatch(/<details[^>]*\bopen/);
    expect(connected).toContain("Connection settings and maintenance");
    const setup = renderToStaticMarkup(
      <HostControls
        {...common}
        panel={{ ...panel, connection: "unavailable", componentsReady: false }}
      />,
    );
    expect(setup).toContain('aria-label="Getting started"');
    expect(setup).toContain("Open Mod Market");
    expect(setup).toContain("Install components");
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
    [{}, "Choose a plugin to start using its features.", true],
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
      if ({ ...panel, ...overrides }.connection === "connected") {
        expect(load).toBeUndefined();
      } else {
        expect(load).toContain('aria-describedby="host-load-guidance"');
        expect(load.includes('disabled=""')).toBe(disabled);
      }
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
  it("offers a manual deployment target without unrelated host configuration", () => {
    const html = renderToStaticMarkup(
      <HostControls {...common} refresh={async () => {}} />,
    );
    for (const label of [
      "Select game directory",
      "Use automatic detection",
      "Host loading method",
      "Proxy loading",
      "Loader loading",
      "Check status",
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
    "NTE_PluginTraining.dll",
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
    const html = renderPlugin("NTE_PluginCustom.dll");
    expect(html).toContain("NTE_PluginCustom.dll");
    expect(html).toContain("Check status");
    expect(html).not.toContain("Load plugin");
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
          plugin.file === "NTE_PluginCustom.dll"
            ? { ...plugin, state: "unload_pending" as const }
            : plugin,
        ),
      },
    ]) {
      const html = renderPlugin("NTE_PluginCustom.dll", snapshot);
      const buttons = html.match(/<button\b[^>]*>[\s\S]*?<\/button>/g)!;
      expect(buttons.length).toBe(2);
      for (const button of buttons.filter(
        (button) => !button.includes("Check status"),
      ))
        expect(button).toContain('disabled=""');
    }
  });
  it("does not enable lifecycle mutations during collection", () => {
    const snapshot = { ...panel, collectorActive: true };
    const html = renderPlugin("NTE_PluginCustom.dll", snapshot);
    const buttons = html.match(/<button\b[^>]*>[\s\S]*?<\/button>/g)!;
    expect(buttons.length).toBe(2);
    for (const button of buttons.filter(
      (button) => !button.includes("Check status"),
    ))
      expect(button).toContain('disabled=""');
    expect(html).toContain(
      "Stop collection before changing plugin lifecycle state.",
    );
  });
});

it("shows manual directory selection only for proxy loading", () => {
  setFrontendLanguage("en");
  const html = renderToStaticMarkup(<HostLoadingControls {...common} />);
  expect(html).toContain("Proxy loading");
  expect(html).toContain("Loader loading");
  expect(html).toContain("managed mods directory");
  expect(html).toContain("Select game directory");
  const loader = renderToStaticMarkup(
    <HostLoadingControls
      {...common}
      panel={{ ...panel, loadingMethod: "loader" }}
    />,
  );
  expect(loader).not.toContain("Select game directory");
  expect(html).not.toContain("Start or connect with Loader");
});

it("shows the selected target and never enables unsupported or unloaded Training controls", () => {
  const html = renderToStaticMarkup(
    <HostLoadingControls
      {...common}
      panel={{ ...panel, gameDirectory: "GAME/Win64" }}
    />,
  );
  expect(html).toContain("GAME/Win64");
  for (const snapshot of [
    { ...panel, capabilities: [4, 5] },
    {
      ...panel,
      plugins: panel.plugins.map((p) =>
        p.file === "NTE_PluginTraining.dll"
          ? { ...p, state: "unloaded" as const }
          : p,
      ),
    },
  ]) {
    const training = renderPlugin("NTE_PluginTraining.dll", snapshot);
    const buttons = training.match(/<button\b[^>]*>[\s\S]*?<\/button>/g)!;
    for (const button of buttons.filter(
      (b) =>
        b.includes('aria-label="Training') ||
        b.includes('aria-label="Enable wooden') ||
        b.includes('aria-label="Disable wooden'),
    ))
      expect(button).toContain('disabled=""');
    expect(training).not.toContain("Refresh account data");
  }
});

describe("user-focused plugin controls", () => {
  beforeEach(() => setFrontendLanguage("en"));
  it.each(["Combat", "User", "Training", "Network"])(
    "keeps %s diagnostic actions available but collapsed",
    (name) => {
      const file = `NTE_Plugin${name}.dll`;
      const html = renderPlugin(file);
      const group = pluginControlGroup(file)!;
      const advanced = html.match(
        /<details[^>]*data-testid="plugin-advanced-controls"[^>]*>[\s\S]*?<\/details>/,
      )![0];
      expect(advanced).not.toMatch(/<details[^>]*\bopen/);
      const main = html.replace(advanced, "");
      for (const action of group.advanced) {
        const label = RELEASE_ACTIONS.find(([key]) => key === action)![2];
        expect(advanced).toContain(label);
        expect(main).not.toContain(label);
      }
      expect(html).toContain("Plugin details");
    },
  );
  it("combines the wooden wheel commands without inventing an enabled state", () => {
    const html = renderPlugin("NTE_PluginTraining.dll");
    expect(html).toContain("Wooden dummy wheel");
    expect(html).toContain('aria-label="Enable wooden dummy wheel"');
    expect(html).toContain('aria-label="Disable wooden dummy wheel"');
    expect(html).not.toContain('role="switch"');
  });
  it("offers loading instead of unloading for an unloaded plugin", () => {
    const snapshot = {
      ...panel,
      plugins: panel.plugins.map((p) => ({ ...p, state: "unloaded" as const })),
    };
    const html = renderPlugin("NTE_PluginCustom.dll", snapshot);
    expect(html).toContain("Load plugin");
    expect(html).not.toContain("Unload plugin");
  });
});
