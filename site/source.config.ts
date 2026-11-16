// SPDX-License-Identifier: Apache-2.0

import { defineConfig, defineDocs } from "fumadocs-mdx/config";
import { pageSchema } from "fumadocs-core/source/schema";
import { heading, remarkDocs, summary } from "./lib/markdown.mjs";

// The site reads the repository's docs/*.md in place; they stay the only copy.
export const docs = defineDocs({
  dir: "../docs",
  docs: {
    files: ["*.md"],
    schema: ({ source }) =>
      pageSchema.extend({
        title: pageSchema.shape.title.default(heading(source)),
        description: pageSchema.shape.description.default(summary(source)),
      }),
    postprocess: { includeProcessedMarkdown: true },
  },
});

export default defineConfig({
  mdxOptions: {
    remarkPlugins: (plugins) => [remarkDocs, ...plugins],
    rehypeCodeOptions: {
      themes: { light: "solarized-light", dark: "gruvbox-dark-medium" },
    },
  },
});
