import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { EMPTY_CURTAIN_FIXTURE } from "@/lib/tauri/empty-curtain-contract.test";
import { setFrontendLanguage } from "@/lib/i18n";
import { EmptyCurtainPage } from "./empty-curtain-page";

const model = vi.hoisted(() => ({
  state: {} as unknown,
  notice: null,
  actionPending: false,
  refreshing: false,
  retry: vi.fn(),
}));
vi.mock("./use-empty-curtain", () => ({ useEmptyCurtain: () => model }));
describe("User plugin equipment inventory", () => {
  it("disables game refresh in capture mode but keeps read and export controls available", () => {
    setFrontendLanguage("en");
    model.state = {
      status: "ready",
      snapshot: { ...EMPTY_CURTAIN_FIXTURE, canOperate: false },
    };
    const html = renderToStaticMarkup(<EmptyCurtainPage />);
    const buttons = html.match(/<button\b[^>]*>[\s\S]*?<\/button>/g) ?? [];
    expect(
      buttons.find((button) => button.includes("Refresh from game")),
    ).toContain("disabled");
    for (const label of [
      "Export for Drive Calculator",
      "Filter",
      "Character Equipment",
    ])
      expect(buttons.find((button) => button.includes(label))).not.toContain(
        'disabled=""',
      );
    expect(html).toContain("Equipment is read-only");
    expect(html).not.toContain("Right-click equipment to manage");
  });
  it("shows confirmation in progress instead of a failed-operation notice", () => {
    setFrontendLanguage("en");
    model.state = {
      status: "ready",
      snapshot: {
        ...EMPTY_CURTAIN_FIXTURE,
        operation: {
          status: "pending",
          messageKey:
            "Equipment operation is awaiting confirmation. Synchronizing inventory; do not repeat it.",
          messageArguments: [],
        },
      },
    };
    const html = renderToStaticMarkup(<EmptyCurtainPage />);
    expect(html).toContain('role="status"');
    expect(html).toContain("Equipment operation is awaiting confirmation");
    expect(html).not.toContain("Equipment connection is unavailable");
  });
  it("keeps a busy spinner visible while retained inventory is refreshing", () => {
    setFrontendLanguage("en");
    model.state = { status: "ready", snapshot: EMPTY_CURTAIN_FIXTURE };
    model.refreshing = true;
    const html = renderToStaticMarkup(<EmptyCurtainPage />);
    expect(html).toContain("Refreshing equipment...");
    expect(html).toContain('aria-busy="true"');
    expect(html).toContain("animate-spin");
    expect(html).toContain("Export for Drive Calculator");
    model.refreshing = false;
    expect(renderToStaticMarkup(<EmptyCurtainPage />)).not.toContain(
      "Refreshing equipment...",
    );
  });
  it("retains inventory, filters, export and character equipment entry points with refresh", () => {
    setFrontendLanguage("en");
    model.state = { status: "ready", snapshot: EMPTY_CURTAIN_FIXTURE };
    const html = renderToStaticMarkup(<EmptyCurtainPage />);
    for (const label of [
      "Refresh from game",
      "Filter",
      "Export for Drive Calculator",
      "Character Equipment",
      "Right-click equipment to manage it through the plugin",
    ])
      expect(html).toContain(label);
    expect(html).not.toContain("Legacy equipment changes are not supported");
  });
  it("does not require packet capture to fill an empty inventory", () => {
    setFrontendLanguage("en");
    model.state = {
      status: "ready",
      snapshot: {
        ...EMPTY_CURTAIN_FIXTURE,
        hasData: false,
        items: [],
        characters: [],
      },
    };
    const html = renderToStaticMarkup(<EmptyCurtainPage />);
    expect(html).toContain("Refresh from the User plugin");
    expect(html).not.toContain(
      "Start capture or import a replay to collect equipment data.",
    );
  });
});
