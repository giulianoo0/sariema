// SPDX-License-Identifier: Apache-2.0

"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import { Gooey } from "@/components/fui/gooey";
import { IconArrowRight } from "@/components/fui/icons-base";
import button from "@/components/fui/button.module.css";
import styles from "./gooey-link.module.css";

/**
 * The primary action: f.UI's primary button with its arrow in a drop beside
 * it. On hover or focus the drop slides in and the two melt into one.
 */
export function GooeyLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <Link href={href} className={styles.link}>
      <Gooey spacing={10} className={styles.goo}>
        <span className={button.button} data-variant="primary" data-size="lg">
          {children}
        </span>
        <span className={styles.drop} aria-hidden>
          <IconArrowRight size={16} />
        </span>
      </Gooey>
    </Link>
  );
}
