// SPDX-License-Identifier: Apache-2.0

import type { ReactNode } from "react";
import { HomeLayout } from "fumadocs-ui/layouts/home";
import { baseOptions } from "@/lib/layout";

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <HomeLayout {...baseOptions} links={[{ text: "Docs", url: "/docs/", active: "nested-url" }]}>
      {children}
    </HomeLayout>
  );
}
