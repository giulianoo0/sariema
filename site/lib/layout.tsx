// SPDX-License-Identifier: Apache-2.0

import type { BaseLayoutProps } from "fumadocs-ui/layouts/shared";
import { Logo } from "@/components/site/logo";
import { SiteThemeToggle } from "@/components/site/theme-toggle";
import { repo } from "./markdown.mjs";

export const baseOptions: BaseLayoutProps = {
  nav: { title: <Logo />, url: "/" },
  githubUrl: repo,
  themeSwitch: { component: <SiteThemeToggle /> },
};
