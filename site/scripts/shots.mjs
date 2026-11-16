// SPDX-License-Identifier: Apache-2.0

// Screenshots of the landing and a docs page at 1440 and 390 px wide, light
// and dark, from a running site (next dev, or `npx serve out`):
//
//   node scripts/shots.mjs http://localhost:3000 <out-dir>
//
// Chromium is Playwright's (`npx playwright install chromium`), started with
// the GPU flags packages/sariema-web's tests use so the demo can draw.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright-core";

const [base = "http://localhost:3000", out = "shots"] = process.argv.slice(2);
mkdirSync(out, { recursive: true });

const pages = [
  { name: "landing", path: "/", full: true },
  { name: "docs-web", path: "/docs/web/", full: false },
];
const widths = [
  { name: "1440", viewport: { width: 1440, height: 900 }, scale: 1, mobile: false },
  { name: "390", viewport: { width: 390, height: 844 }, scale: 2, mobile: true },
];

const browser = await chromium.launch({
  args: ["--use-angle=metal", "--enable-gpu", "--ignore-gpu-blocklist", "--enable-unsafe-webgpu"],
});
const report = [];
try {
  for (const theme of ["light", "dark"]) {
    for (const w of widths) {
      const context = await browser.newContext({
        viewport: w.viewport,
        deviceScaleFactor: w.scale,
        isMobile: w.mobile,
        hasTouch: w.mobile,
        colorScheme: theme,
      });
      await context.addInitScript((t) => localStorage.setItem("theme", t), theme);
      const page = await context.newPage();
      for (const p of pages) {
        await page.goto(base + p.path, { waitUntil: "networkidle" });
        if (p.name === "landing") {
          await page.getByText(/^Live · /).waitFor({ timeout: 20000 }).catch(() => {});
        }
        // Scroll through so the in-view entrances run, then let them finish.
        const height = await page.evaluate(() => document.documentElement.scrollHeight);
        for (let y = 0; y < height; y += w.viewport.height / 2) {
          await page.evaluate((top) => window.scrollTo(0, top), y);
          await page.waitForTimeout(120);
        }
        await page.evaluate(() => window.scrollTo(0, 0));
        await page.waitForTimeout(2200);
        // Content past the right edge, scrolled or clipped (code scrolls inside its block).
        const overflow = await page.evaluate(() => {
          const blocks = [...document.querySelectorAll("h1, h2, h3, p, figure, section, header, footer, nav")];
          const past = blocks.filter((e) => e.getBoundingClientRect().right > window.innerWidth + 1).length;
          return { page: document.documentElement.scrollWidth - window.innerWidth, blocks: past };
        });
        const file = join(out, `${p.name}-${w.name}-${theme}.png`);
        await page.screenshot({ path: file, fullPage: p.full });
        const live = p.name === "landing" ? await page.getByText(/^Live · /).textContent().catch(() => null) : null;
        report.push({ file, overflow, ...(live !== null ? { demo: live } : {}) });
      }
      await context.close();
    }
  }
} finally {
  await browser.close();
}
console.log(JSON.stringify(report, null, 1));
