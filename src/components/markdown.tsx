import { memo, type ReactNode } from "react";
import { Link } from "react-router-dom";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import rehypeHighlight from "rehype-highlight";
import "katex/dist/katex.min.css";

import type { Citation } from "@/types/api";

/**
 * Markdown renderer used for AI answers and summaries.
 *
 * Security posture: `react-markdown` does not evaluate raw HTML unless
 * `rehype-raw` is added (we deliberately do not), so model output cannot inject
 * markup or scripts. Links are forced to open externally by the caller.
 */

/** Minimal mdast shape — we only touch text and link nodes. */
type MdNode = {
  type: string;
  value?: string;
  url?: string;
  children?: MdNode[];
};

const CITATION_MARKER = /\[S(\d+)\]/g;

/**
 * Turn `[S1]` markers in the answer into links to the cited page.
 *
 * The marker is only linked when the message really has a citation with that
 * label, so a hallucinated `[S9]` stays plain text instead of becoming a
 * plausible-looking jump. Code spans/blocks are skipped.
 */
function remarkCitationLinks(citations: Citation[] | undefined) {
  const byLabel = new Map<number, Citation>();
  for (const citation of citations ?? []) {
    byLabel.set(citation.ordinal, citation);
  }

  return () => (tree: MdNode) => {
    if (byLabel.size === 0) return;

    const splitText = (value: string): MdNode[] => {
      const out: MdNode[] = [];
      let cursor = 0;
      for (const match of value.matchAll(CITATION_MARKER)) {
        const index = match.index ?? 0;
        const label = Number(match[1]);
        const citation = byLabel.get(label);
        const page = citation?.page ?? citation?.slide ?? null;
        if (!citation || page === null) {
          continue;
        }
        if (index > cursor) {
          out.push({ type: "text", value: value.slice(cursor, index) });
        }
        out.push({
          type: "link",
          url: `#/materials/${citation.documentId}?page=${page}`,
          children: [{ type: "text", value: `S${label}` }],
        });
        cursor = index + match[0].length;
      }
      if (out.length === 0) return [{ type: "text", value }];
      if (cursor < value.length) {
        out.push({ type: "text", value: value.slice(cursor) });
      }
      return out;
    };

    const walk = (node: MdNode) => {
      if (!node.children) return;
      const next: MdNode[] = [];
      for (const child of node.children) {
        if (child.type === "text" && typeof child.value === "string") {
          next.push(...splitText(child.value));
          continue;
        }
        if (child.type !== "link" && child.type !== "code" && child.type !== "inlineCode") {
          walk(child);
        }
        next.push(child);
      }
      node.children = next;
    };

    walk(tree);
  };
}

export const Markdown = memo(function Markdown({
  content,
  className,
  citations,
}: {
  content: string;
  className?: string;
  citations?: Citation[];
}) {
  return (
    <div className={className ? `markdown-body ${className}` : "markdown-body"}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath, remarkCitationLinks(citations)]}
        rehypePlugins={[
          rehypeKatex,
          [rehypeHighlight, { detect: false, ignoreMissing: true }],
        ]}
        components={{
          // In-page jumps (our own [S1] chips) stay inside the app; anything
          // else is an external link and must not navigate the webview.
          a: ({ children, href }) =>
            href && href.startsWith("#/") ? (
              <Link
                to={href.slice(1)}
                className="mx-0.5 inline-flex items-baseline rounded bg-brand-500/15 px-1 text-[0.72em] font-semibold text-brand-700 no-underline hover:bg-brand-500/25 dark:text-brand-300"
                title="跳转到引用的页码"
              >
                {children}
              </Link>
            ) : (
              <a href={href} target="_blank" rel="noreferrer noopener">
                {children}
              </a>
            ),
          table: ({ children }) => (
            <div className="overflow-x-auto">
              <table>{children}</table>
            </div>
          ),
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
});

/** Inline wrapper that keeps long math scrollable on narrow screens. */
export function MarkdownBlock({ children }: { children: ReactNode }) {
  return <div className="markdown-body overflow-x-auto">{children}</div>;
}
