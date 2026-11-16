// SPDX-License-Identifier: Apache-2.0

"use client";

import { useSyncExternalStore } from "react";
import { useTheme } from "next-themes";
import { ThemeToggle } from "@/components/fui/theme-toggle";

const never = () => () => {};

/** f.UI's sweep toggle, with next-themes keeping the choice. */
export function SiteThemeToggle() {
  const { resolvedTheme, setTheme } = useTheme();
  // The server cannot know the theme: hydrate as it rendered, then follow next-themes.
  const hydrated = useSyncExternalStore(never, () => true, () => false);
  const theme = hydrated && (resolvedTheme === "dark" || resolvedTheme === "light") ? resolvedTheme : undefined;
  const label = theme === "dark" ? "Switch to light" : "Switch to dark";
  return <ThemeToggle theme={theme} onThemeChange={setTheme} aria-label={label} data-tip={label} />;
}
