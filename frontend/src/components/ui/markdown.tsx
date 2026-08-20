import { Fragment, type ComponentProps, type ReactNode } from "react";

import { cn } from "@/lib/utils";

interface MarkdownProps extends Omit<ComponentProps<"div">, "children"> {
  children: string;
}

const INLINE_TOKEN =
  /(`[^`\n]+`|\*\*[^*\n]+\*\*|__[^_\n]+__|~~[^~\n]+~~|\[[^\]\n]+\]\([^\s)]+\)|\*[^*\n]+\*|_[^_\n]+_)/g;

const HEADING_CLASSES = [
  "text-base font-semibold",
  "text-sm font-semibold",
  "text-sm font-medium",
  "text-xs font-semibold uppercase tracking-wide",
  "text-xs font-semibold uppercase tracking-wide",
  "text-xs font-semibold uppercase tracking-wide",
] as const;

export function Markdown({ children, className, ...props }: MarkdownProps) {
  const lines = children.trim().replaceAll("\r\n", "\n").split("\n");
  const blocks: ReactNode[] = [];

  for (let index = 0; index < lines.length;) {
    const line = lines[index] ?? "";
    if (!line.trim()) {
      index += 1;
      continue;
    }

    const fence = line.match(/^```([\w-]*)\s*$/);
    if (fence) {
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !/^```\s*$/.test(lines[index] ?? "")) {
        code.push(lines[index] ?? "");
        index += 1;
      }
      if (index < lines.length) index += 1;
      const language = fence[1];
      blocks.push(
        <pre
          className="mt-2 overflow-x-auto rounded-lg border bg-background/70 p-3 font-mono text-[0.6875rem] leading-relaxed text-foreground first:mt-0"
          key={`code-${index}`}
        >
          <code className={language ? `language-${language}` : undefined}>
            {code.join("\n")}
          </code>
        </pre>,
      );
      continue;
    }

    const heading = line.match(/^(#{1,6})\s+(.+)$/);
    if (heading) {
      const level = heading[1].length;
      const Tag = `h${level}` as "h1" | "h2" | "h3" | "h4" | "h5" | "h6";
      blocks.push(
        <Tag
          className={cn(
            "mt-3 text-foreground first:mt-0",
            HEADING_CLASSES[level - 1],
          )}
          key={`heading-${index}`}
        >
          {renderInline(heading[2], `heading-${index}`)}
        </Tag>,
      );
      index += 1;
      continue;
    }

    if (/^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/.test(line)) {
      blocks.push(<hr className="my-3 border-border" key={`rule-${index}`} />);
      index += 1;
      continue;
    }

    if (/^\s*>\s?/.test(line)) {
      const quote: string[] = [];
      while (index < lines.length) {
        const match = (lines[index] ?? "").match(/^\s*>\s?(.*)$/);
        if (!match) break;
        quote.push(match[1]);
        index += 1;
      }
      blocks.push(
        <blockquote
          className="mt-2 border-l-2 border-border pl-3 text-muted-foreground italic first:mt-0"
          key={`quote-${index}`}
        >
          {renderInlineLines(quote, `quote-${index}`)}
        </blockquote>,
      );
      continue;
    }

    const unordered = line.match(/^\s*[-+*]\s+(.+)$/);
    if (unordered) {
      const items: string[] = [];
      while (index < lines.length) {
        const match = (lines[index] ?? "").match(/^\s*[-+*]\s+(.+)$/);
        if (!match) break;
        items.push(match[1]);
        index += 1;
      }
      blocks.push(
        <ul
          className="mt-2 list-disc space-y-1 pl-5 marker:text-muted-foreground first:mt-0"
          key={`unordered-${index}`}
        >
          {items.map((item, itemIndex) => (
            <li key={itemIndex}>{renderInline(item, `ul-${itemIndex}`)}</li>
          ))}
        </ul>,
      );
      continue;
    }

    const ordered = line.match(/^\s*\d+[.)]\s+(.+)$/);
    if (ordered) {
      const items: string[] = [];
      while (index < lines.length) {
        const match = (lines[index] ?? "").match(/^\s*\d+[.)]\s+(.+)$/);
        if (!match) break;
        items.push(match[1]);
        index += 1;
      }
      blocks.push(
        <ol
          className="mt-2 list-decimal space-y-1 pl-5 marker:text-muted-foreground first:mt-0"
          key={`ordered-${index}`}
        >
          {items.map((item, itemIndex) => (
            <li key={itemIndex}>{renderInline(item, `ol-${itemIndex}`)}</li>
          ))}
        </ol>,
      );
      continue;
    }

    const paragraph: string[] = [];
    while (index < lines.length && (lines[index] ?? "").trim()) {
      const candidate = lines[index] ?? "";
      if (paragraph.length > 0 && startsBlock(candidate)) break;
      paragraph.push(candidate);
      index += 1;
    }
    blocks.push(
      <p className="mt-2 first:mt-0" key={`paragraph-${index}`}>
        {renderInlineLines(paragraph, `paragraph-${index}`)}
      </p>,
    );
  }

  return (
    <div
      data-slot="markdown"
      className={cn("min-w-0 break-words select-text", className)}
      {...props}
    >
      {blocks}
    </div>
  );
}

function startsBlock(line: string): boolean {
  return (
    /^```/.test(line) ||
    /^(?:#{1,6})\s+/.test(line) ||
    /^\s*>\s?/.test(line) ||
    /^\s*[-+*]\s+/.test(line) ||
    /^\s*\d+[.)]\s+/.test(line) ||
    /^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/.test(line)
  );
}

function renderInlineLines(lines: string[], keyPrefix: string): ReactNode[] {
  return lines.flatMap((line, lineIndex) => {
    const content = renderInline(line, `${keyPrefix}-${lineIndex}`);
    return lineIndex === lines.length - 1
      ? content
      : [...content, <br key={`${keyPrefix}-break-${lineIndex}`} />];
  });
}

function renderInline(text: string, keyPrefix: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let cursor = 0;

  for (const match of text.matchAll(INLINE_TOKEN)) {
    const token = match[0];
    const start = match.index;
    if (start > cursor) nodes.push(text.slice(cursor, start));

    const key = `${keyPrefix}-${start}`;
    if (token.startsWith("`")) {
      nodes.push(
        <code
          className="rounded bg-background/80 px-1 py-0.5 font-mono text-[0.9em] text-foreground"
          key={key}
        >
          {token.slice(1, -1)}
        </code>,
      );
    } else if (token.startsWith("**") || token.startsWith("__")) {
      nodes.push(
        <strong className="font-semibold text-foreground" key={key}>
          {renderInline(token.slice(2, -2), key)}
        </strong>,
      );
    } else if (token.startsWith("~~")) {
      nodes.push(<del key={key}>{renderInline(token.slice(2, -2), key)}</del>);
    } else if (token.startsWith("[")) {
      const link = token.match(/^\[([^\]]+)\]\(([^\s)]+)\)$/);
      const href = link ? safeHref(link[2]) : null;
      nodes.push(
        href === null ? (
          <Fragment key={key}>{link?.[1] ?? token}</Fragment>
        ) : (
          <a
            className="font-medium text-primary underline decoration-primary/50 underline-offset-2 hover:decoration-primary"
            href={href}
            key={key}
            rel="noreferrer"
            target="_blank"
          >
            {renderInline(link?.[1] ?? "", key)}
          </a>
        ),
      );
    } else {
      nodes.push(<em key={key}>{renderInline(token.slice(1, -1), key)}</em>);
    }
    cursor = start + token.length;
  }

  if (cursor < text.length) nodes.push(text.slice(cursor));
  return nodes;
}

function safeHref(value: string): string | null {
  return /^(?:https?:|mailto:|#)/i.test(value) ? value : null;
}
