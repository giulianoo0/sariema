// SPDX-License-Identifier: Apache-2.0

"use client";

import { useState, type ReactNode } from "react";
import { Segmented } from "@/components/fui/segmented";
import styles from "./code-tabs.module.css";

export interface CodeTab {
  value: string;
  label: string;
  caption: ReactNode;
  code: ReactNode;
}

/** f.UI's segmented control choosing between code blocks highlighted at build. */
export function CodeTabs({ tabs }: { tabs: CodeTab[] }) {
  const [value, setValue] = useState(tabs[0].value);
  const tab = tabs.find((t) => t.value === value) ?? tabs[0];
  return (
    <div className={styles.tabs}>
      <div className={styles.bar}>
        <Segmented label="Language" value={value} onChange={setValue} options={tabs.map(({ value, label }) => ({ value, label }))} />
        <p className={styles.caption}>{tab.caption}</p>
      </div>
      {tabs.map((t) => (
        <div key={t.value} hidden={t.value !== value}>
          {t.code}
        </div>
      ))}
    </div>
  );
}
