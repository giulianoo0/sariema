// SPDX-License-Identifier: Apache-2.0

import type { ReactNode } from "react";
import { DocsLayout } from "fumadocs-ui/layouts/docs";
import { baseOptions } from "@/lib/layout";
import { docsTree } from "@/lib/source";

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <DocsLayout tree={docsTree()} {...baseOptions}>
      {children}
    </DocsLayout>
  );
}
