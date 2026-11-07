import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { setFrontendLanguage } from "@/lib/i18n";
import type { EffectModifier } from "@/lib/tauri/generated/EffectModifier";
import { EffectModifierValues } from "./effect-modifier-values";

it("shows frozen percentages, factors and real zero without changing the observation", () => {
  setFrontendLanguage("zh-CN");
  const values: EffectModifier[] = [
    { attribute: "AtkUp", operation: "add_base", magnitude: 0.125 },
    { attribute: "AtkAdd", operation: "add_final", magnitude: 0 },
    { attribute: "AtkUp", operation: "multiply_compound", magnitude: 1.2 },
  ];
  const html = renderToStaticMarkup(
    <EffectModifierValues modifiers={{ status: "recorded", values }} />,
  );
  expect(html).toContain("攻击力百分比");
  expect(html).toContain("12.5%");
  expect(html).toContain(">0<");
  expect(html).toContain(">1.2<");
  expect(html).not.toContain("120%");
  expect(values[0].magnitude).toBe(0.125);
});
it("distinguishes old missing data from an observed empty modifier list", () => {
  setFrontendLanguage("zh-CN");
  expect(
    renderToStaticMarkup(
      <EffectModifierValues modifiers={{ status: "unavailable" }} />,
    ),
  ).toContain("读取修正量失败");
  expect(
    renderToStaticMarkup(<EffectModifierValues modifiers={null} />),
  ).toContain("旧历史不会");
  expect(
    renderToStaticMarkup(
      <EffectModifierValues modifiers={{ status: "recorded", values: [] }} />,
    ),
  ).toContain("没有直接属性修正项");
});
