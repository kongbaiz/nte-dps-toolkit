import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import { setFrontendLanguage } from "@/lib/i18n";
import type { PluginPanel } from "@/lib/tauri/generated/PluginPanel";
import { HostLoadingControls } from "./host-loading-controls";
import { HostControls } from "./host-controls";
import { PluginControls } from "./plugin-controls";

const panel: PluginPanel = {
  loadingMethod: "proxy",
  gameDirectory: null,
  connectionIdentity: "123:456",
  collectorActive: false,
  mode: "plugin",
  connection: "connected",
  componentsReady: true,
  capabilities: [2, 4, 5, 6, 11, 12],
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
  it("gives unknown plugins their own lifecycle page without guessing capabilities", () => {
    const html = renderPlugin("NTE_PluginCustom.dll");
    expect(html).toContain("NTE_PluginCustom.dll");
    expect(html).not.toContain("Load plugin");
    expect(html).toContain("Unload plugin");
    expect(html).not.toContain("Reload");
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
      for (const button of buttons.filter(
        (button) =>
          button.includes("Load plugin") || button.includes("Unload plugin"),
      ))
        expect(button).toContain('disabled=""');
    }
  });
  it("does not enable lifecycle mutations during collection", () => {
    const snapshot = { ...panel, collectorActive: true };
    const html = renderPlugin("NTE_PluginCustom.dll", snapshot);
    const buttons = html.match(/<button\b[^>]*>[\s\S]*?<\/button>/g)!;
    for (const button of buttons.filter(
      (button) =>
        button.includes("Load plugin") || button.includes("Unload plugin"),
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

it("offers loading instead of unloading for an unloaded plugin", () => {
  const snapshot = {
    ...panel,
    plugins: panel.plugins.map((p) => ({ ...p, state: "unloaded" as const })),
  };
  const html = renderPlugin("NTE_PluginCustom.dll", snapshot);
  expect(html).toContain("Load plugin");
  expect(html).toContain("Load the plugin to view its controls.");
});

it("offers proxy removal only for proxy mode and blocks it during a live connection", () => {
  setFrontendLanguage("en");
  const connected = renderToStaticMarkup(<HostLoadingControls {...common} />);
  const button = connected
    .match(/<button\b[^>]*>[\s\S]*?<\/button>/g)!
    .find((b) => b.includes("Uninstall proxy"))!;
  expect(button).toContain('disabled=""');
  const stopped = renderToStaticMarkup(
    <HostLoadingControls
      {...common}
      panel={{ ...panel, connection: "unavailable" }}
    />,
  );
  expect(
    stopped
      .match(/<button\b[^>]*>[\s\S]*?<\/button>/g)!
      .find((b) => b.includes("Uninstall proxy")),
  ).not.toContain('disabled=""');
  expect(
    renderToStaticMarkup(
      <HostLoadingControls
        {...common}
        panel={{ ...panel, loadingMethod: "loader" }}
      />,
    ),
  ).not.toContain("Uninstall proxy");
});
