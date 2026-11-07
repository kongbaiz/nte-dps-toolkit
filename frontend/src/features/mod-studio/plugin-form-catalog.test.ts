import { beforeEach, expect, it, vi } from "vitest";
import { invokeCommand } from "@/lib/tauri/command-error";
import type { PluginFormSnapshot } from "@/lib/tauri/generated/PluginFormSnapshot";
import { pluginFormTitle, usePluginFormCatalog } from "./plugin-form-catalog";

vi.mock("@/lib/tauri/command-error", () => ({
  invokeCommand: vi.fn(),
  toCommandError: (value: unknown) => value,
}));
beforeEach(() => {
  vi.mocked(invokeCommand).mockReset();
  usePluginFormCatalog.setState({ entries: {} });
});

it("uses provider titles and keeps a late old-instance response out of the current menu", async () => {
  const snapshot = (owner: string, title: string): PluginFormSnapshot => ({
    owner,
    form: {
      version: 1,
      title,
      schema: {},
      uischema: {},
      data: {},
      actions: [],
      translations: { "zh-CN": { [title]: "提供者功能名称" } },
    },
  });
  let finishOld!: (value: PluginFormSnapshot) => void;
  vi.mocked(invokeCommand).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishOld = resolve as typeof finishOld;
      }),
  );
  const old = usePluginFormCatalog
    .getState()
    .load("opaque.dll", "session", "1");
  vi.mocked(invokeCommand).mockResolvedValueOnce(snapshot("2", "New feature"));
  await usePluginFormCatalog.getState().load("opaque.dll", "session", "2");
  finishOld(snapshot("1", "Old feature"));
  await old;
  const current =
    usePluginFormCatalog.getState().entries["opaque.dll"].snapshot;
  expect(current?.owner).toBe("2");
  expect(pluginFormTitle(current, "en")).toBe("New feature");
  expect(pluginFormTitle(current, "zh-CN")).toBe("提供者功能名称");
});
