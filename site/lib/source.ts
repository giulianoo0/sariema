// SPDX-License-Identifier: Apache-2.0

import { docs } from "fumadocs-mdx:collections/server";
import { loader } from "fumadocs-core/source";
import type * as PageTree from "fumadocs-core/page-tree";
import { sections } from "./sections.mjs";

export const source = loader({
  baseUrl: "/docs",
  source: docs.toFumadocsSource(),
});

type DocPage = ReturnType<typeof source.getPages>[number];

/** The sections in order, then a last one with any doc no section lists. */
export function docsSections(): { title: string; docs: { page: DocPage; label: string }[] }[] {
  const pages = new Map(source.getPages().map((p) => [p.slugs.join("/"), p]));
  const listed = sections.map((section) => ({
    title: section.title,
    docs: section.docs.flatMap(([slug, label]) => {
      const page = pages.get(slug);
      pages.delete(slug);
      return page ? [{ page, label }] : [];
    }),
  }));
  const rest = [...pages.values()].map((page) => ({ page, label: page.data.title }));
  return [...listed, { title: "More", docs: rest }].filter((s) => s.docs.length);
}

export function docsTree(): PageTree.Root {
  return {
    name: "Docs",
    children: docsSections().flatMap((s): PageTree.Node[] => [
      { type: "separator", name: s.title },
      ...s.docs.map((d): PageTree.Item => ({ type: "page", name: d.label, url: d.page.url })),
    ]),
  };
}
