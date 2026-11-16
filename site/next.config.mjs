// SPDX-License-Identifier: Apache-2.0

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createMDX } from "fumadocs-mdx/next";

const repo = join(dirname(fileURLToPath(import.meta.url)), "..");

/** @type {import('next').NextConfig} */
const config = {
  output: "export",
  trailingSlash: true,
  images: { unoptimized: true },
  devIndicators: false,
  agentRules: false,
  reactStrictMode: true,
  // docs/ and assets/ live in the repository, outside this folder.
  turbopack: { root: repo },
  outputFileTracingRoot: repo,
};

export default createMDX()(config);
