// SPDX-License-Identifier: Apache-2.0

import { visit } from "unist-util-visit";

export const repo = "https://github.com/giulianoo0/sariema";

/** The text of the first `# ` heading: the docs keep their title there. */
export function heading(source) {
  return /^# (.+)$/m.exec(source)?.[1].trim() ?? "";
}

/** The first paragraph after the title, as plain text, for search and meta tags. */
export function summary(source) {
  const body = source.slice(source.search(/^# /m)).split("\n").slice(1).join("\n");
  const paragraph = body.split(/\n\s*\n/).find((p) => /^[A-Za-z`*_[]/.test(p.trim())) ?? "";
  const text = paragraph
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[`*_]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  return text.length > 220 ? `${text.slice(0, text.lastIndexOf(" ", 217))}…` : text;
}

/**
 * Where a link in docs/ points on the site: another doc becomes its page,
 * a file of the repository its page on GitHub.
 */
export function rewriteLink(url) {
  if (/^([a-z]+:|#|\/)/i.test(url)) return url;
  const doc = /^(?:\.\/)?([\w-]+)\.md(#.*)?$/.exec(url);
  if (doc) return `/docs/${doc[1]}/${doc[2] ?? ""}`;
  const path = new URL(url, `${repo}/blob/main/docs/`).pathname;
  return `https://github.com${path}`;
}

/** Drops the title heading (the page renders it) and rewrites relative links. */
export function remarkDocs() {
  return (tree) => {
    const title = tree.children.findIndex((n) => n.type === "heading" && n.depth === 1);
    if (title !== -1) tree.children.splice(title, 1);
    visit(tree, ["link", "definition"], (node) => {
      node.url = rewriteLink(node.url);
    });
  };
}
