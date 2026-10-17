import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { FormRow } from "./form-row";
import {
  PageHeader,
  PageTitle,
  PageDescription,
  PageScrollArea,
  PreferencesGrid,
} from "./page-layout";
import { Input, NativeSelect } from "@/components/ui/form-controls";

describe("shared UI primitives", () => {
  it("preserves semantic headings, content and explicit overrides", () => {
    const html = renderToStaticMarkup(
      <>
        <PageHeader className="p-2">
          <PageTitle>Settings</PageTitle>
          <PageDescription>Preferences</PageDescription>
        </PageHeader>
        <PageScrollArea>
          <PreferencesGrid>
            <p>Content</p>
          </PreferencesGrid>
        </PageScrollArea>
      </>,
    );
    expect(html).toContain("<header");
    expect(html).toContain("<h1");
    expect(html).toContain("ui-page-header p-2");
    expect(html).toContain("Content");
  });

  it("associates field group labels and descriptions without nesting labels", () => {
    const html = renderToStaticMarkup(
      <FormRow label="Language" description="Choose a language">
        <NativeSelect aria-label="Language" defaultValue="en">
          <option value="en">English</option>
        </NativeSelect>
      </FormRow>,
    );
    const label = html.match(/aria-labelledby="([^"]+)"/)?.[1];
    const description = html.match(/aria-describedby="([^"]+)"/)?.[1];
    expect(label).toBeTruthy();
    expect(description).toBeTruthy();
    expect(html).toContain(`id="${label}"`);
    expect(html).toContain(`id="${description}"`);
    expect(html).toContain('role="group"');
    expect(html).toContain('aria-label="Language"');
    expect(html).toContain('value="en" selected=""');
  });

  it("forwards native input validation, disabled state and local sizing", () => {
    const html = renderToStaticMarkup(
      <Input
        aria-label="Width"
        type="number"
        min={280}
        max={960}
        disabled
        aria-invalid="true"
        className="w-24"
        defaultValue={380}
      />,
    );
    for (const value of [
      'type="number"',
      'min="280"',
      'max="960"',
      'disabled=""',
      'aria-invalid="true"',
      'value="380"',
      "ui-control w-24",
    ])
      expect(html).toContain(value);
  });
});
