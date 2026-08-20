import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { Markdown } from "./markdown";

describe("Markdown", () => {
  it("renders the release-note Markdown used by update surfaces", () => {
    const html = renderToStaticMarkup(
      <Markdown>{`## Improvements

- **Faster** capture
- Supports \`JSON\` import

1. Download
2. Restart

> Existing settings are preserved.

[Release page](https://example.com/release)`}</Markdown>,
    );

    expect(html).toContain("<h2");
    expect(html).toContain("<ul");
    expect(html).toContain("<ol");
    expect(html).toContain("<strong");
    expect(html).toContain("<code");
    expect(html).toContain("<blockquote");
    expect(html).toContain('href="https://example.com/release"');
  });

  it("keeps raw HTML inert and drops unsafe link targets", () => {
    const html = renderToStaticMarkup(
      <Markdown>{`<script>alert("xss")</script>

[unsafe](javascript:alert(1))`}</Markdown>,
    );

    expect(html).toContain("&lt;script&gt;");
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("javascript:");
    expect(html).toContain("unsafe");
  });

  it("renders fenced code without interpreting its contents", () => {
    const html = renderToStaticMarkup(
      <Markdown>{`\`\`\`ts
const value = "<tag>";
\`\`\``}</Markdown>,
    );

    expect(html).toContain('class="language-ts"');
    expect(html).toContain("&lt;tag&gt;");
    expect(html).not.toContain("<tag>");
  });
});
