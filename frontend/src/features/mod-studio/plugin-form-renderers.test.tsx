import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PluginForm } from "@/lib/tauri/generated/PluginForm";
import { PluginFormRenderer } from "./plugin-form-renderers";

const switches = vi.hoisted(() => new Map<string, (value: boolean) => void>());
vi.mock("@/components/ui/switch", () => ({
  Switch: (props: {
    "aria-label": string;
    checked: boolean;
    disabled: boolean;
    onCheckedChange: (value: boolean) => void;
  }) => {
    switches.set(props["aria-label"], props.onCheckedChange);
    return (
      <button
        role="switch"
        aria-label={props["aria-label"]}
        aria-checked={props.checked}
        disabled={props.disabled}
      />
    );
  },
}));
beforeEach(() => switches.clear());

describe("plugin-owned JSON Forms", () => {
  it("renders an unfamiliar plugin and applies JSON Forms enablement rules", () => {
    const form: PluginForm = {
      version: 1,
      title: "Custom inventory tool",
      translations: {},
      data: {},
      schema: {
        type: "object",
        properties: {
          armed: { type: "boolean", title: "Allow request" },
          amount: { type: "number", title: "Amount" },
        },
      },
      uischema: {
        type: "Group",
        label: "Provider section",
        elements: [
          { type: "Control", scope: "#/properties/armed" },
          { type: "Control", scope: "#/properties/amount" },
          {
            type: "Button",
            options: { action: "apply" },
            rule: {
              effect: "DISABLE",
              condition: {
                scope: "#/properties/armed",
                schema: { const: false },
              },
            },
          },
        ],
      },
      actions: [
        {
          id: "apply",
          label: "Provider action",
          confirmation: "Confirm request?",
          requiresIdle: false,
          dev: false,
        },
      ],
    };
    for (const armed of [false, true]) {
      const html = renderToStaticMarkup(
        <PluginFormRenderer
          form={form}
          data={{ armed, amount: 0 }}
          disabled={false}
          capturing={false}
          translate={(key) => key}
          invoke={() => {}}
          onChange={() => {}}
        />,
      );
      expect(html).toContain("Provider section");
      expect(html).toContain('aria-label="Amount"');
      expect(html).toContain('value="0"');
      const action = html.match(/<button\b[^>]*>Provider action<\/button>/)![0];
      expect(action.includes('disabled=""')).toBe(!armed);
    }
  });

  it("renders editable repeated parameters through the same schema engine", () => {
    const form: PluginForm = {
      version: 1,
      title: "Custom",
      translations: {},
      data: {},
      actions: [],
      schema: {
        type: "object",
        properties: {
          rows: {
            type: "array",
            title: "Presets",
            items: {
              type: "object",
              properties: {
                attribute: { type: "integer", title: "Attribute" },
                value: { type: "number", title: "Value" },
              },
            },
          },
        },
      },
      uischema: { type: "Control", scope: "#/properties/rows" },
    };
    const html = renderToStaticMarkup(
      <PluginFormRenderer
        form={form}
        data={{ rows: [{ attribute: 0, value: -0.25 }] }}
        disabled={false}
        capturing={false}
        translate={(key) => key}
        invoke={() => {}}
        onChange={() => {}}
      />,
    );
    expect(html).toContain('aria-label="Attribute"');
    expect(html).toContain('value="-0.25"');
    expect(html).toContain('value="0"');
  });
  it("sends an instant switch change with the new value and preserves the other settings", () => {
    const form: PluginForm = {
      version: 1,
      title: "Provider",
      translations: {},
      data: {},
      actions: [
        {
          id: "toggle",
          label: "Toggle",
          confirmation: null,
          requiresIdle: false,
          dev: false,
        },
      ],
      schema: {
        type: "object",
        properties: {
          enabled: { type: "boolean", title: "Instant setting" },
          other: { type: "boolean" },
        },
      },
      uischema: {
        type: "Control",
        scope: "#/properties/enabled",
        options: { action: "toggle" },
      },
    };
    const invoke = vi.fn();
    renderToStaticMarkup(
      <PluginFormRenderer
        form={form}
        data={{ enabled: false, other: true }}
        disabled={false}
        capturing={false}
        translate={(key) => key}
        invoke={invoke}
        onChange={() => {}}
      />,
    );
    expect(invoke).not.toHaveBeenCalled();
    switches.get("Instant setting")!(true);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("toggle", {
      enabled: true,
      other: true,
    });
  });
});
