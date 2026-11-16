// SPDX-License-Identifier: Apache-2.0

// Writes the brand files made from the hand-drawn mark and Geist SemiBold:
// wordmark.svg, lockup.svg, lockup-dark.svg, favicon-32.png and og.png in
// assets/brand/. The wordmark is outlined so it renders without the font.
//
//   node scripts/brand.mjs
//
// The PNGs are drawn by Playwright's Chromium (`npx playwright install chromium`).

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import opentype from "opentype.js";
import { chromium } from "playwright-core";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const brand = join(site, "..", "assets", "brand");
const fonts = join(site, "node_modules", "geist", "dist", "fonts", "geist-sans");
const font = opentype.loadSync(join(fonts, "Geist-SemiBold.ttf"));

const ink = { light: "#1C1814", dark: "#EFE3CC" };
const spdx = "<!-- SPDX-License-Identifier: Apache-2.0 -->\n";
const tracking = -0.035;

/** "sariema" set at `size` with the brand's tracking, baseline at y = 0. */
function word(size) {
  const scale = size / font.unitsPerEm;
  const glyphs = font.stringToGlyphs("sariema");
  const path = new opentype.Path();
  let x = 0;
  glyphs.forEach((glyph, i) => {
    path.extend(glyph.getPath(x, 0, size));
    const kern = i + 1 < glyphs.length ? font.getKerningValue(glyph, glyphs[i + 1]) : 0;
    x += (glyph.advanceWidth + kern) * scale + tracking * size;
  });
  const box = path.getBoundingBox();
  return { d: path.toPathData(2), x1: box.x1, x2: box.x2, y1: box.y1, y2: box.y2 };
}

const markSvg = readFileSync(join(brand, "mark.svg"), "utf8");
const markBody = markSvg.slice(markSvg.indexOf(">", markSvg.indexOf("<svg")) + 1, markSvg.lastIndexOf("</svg>"));

function wordmark() {
  const w = word(100);
  const width = w.x2 - w.x1;
  const height = w.y2 - w.y1;
  return `${spdx}<svg xmlns="http://www.w3.org/2000/svg" viewBox="${w.x1.toFixed(2)} ${w.y1.toFixed(2)} ${width.toFixed(2)} ${height.toFixed(2)}" role="img" aria-label="Sariema">
  <path d="${w.d}" fill="${ink.light}"/>
</svg>
`;
}

/** The mark (128 units) and the word, its x-height centred on the sun. */
function lockup(color) {
  const size = 84;
  const gap = 30;
  const w = word(size);
  const xHeight = (font.tables.os2.sxHeight / font.unitsPerEm) * size;
  const baseline = 64 + xHeight / 2;
  const left = 128 + gap - w.x1;
  const width = 128 + gap + (w.x2 - w.x1);
  return `${spdx}<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width.toFixed(2)} 128" role="img" aria-label="Sariema">${markBody}  <path transform="translate(${left.toFixed(2)} ${baseline.toFixed(2)})" d="${w.d}" fill="${color}"/>
</svg>
`;
}

writeFileSync(join(brand, "wordmark.svg"), wordmark());
writeFileSync(join(brand, "lockup.svg"), lockup(ink.light));
writeFileSync(join(brand, "lockup-dark.svg"), lockup(ink.dark));

const data = (file, type) => `data:${type};base64,${readFileSync(file).toString("base64")}`;

const og = `<!doctype html>
<html><head><style>
@font-face { font-family: Geist; src: url("${data(join(fonts, "Geist-SemiBold.ttf"), "font/ttf")}"); font-weight: 600; }
@font-face { font-family: Geist; src: url("${data(join(fonts, "Geist-Regular.ttf"), "font/ttf")}"); font-weight: 400; }
html, body { margin: 0; }
body { width: 1200px; height: 630px; overflow: hidden; position: relative; background: #17130F; color: #EFE3CC; font-family: Geist; }
.glow { position: absolute; right: -180px; top: -60px; width: 820px; height: 820px; border-radius: 50%;
  background: radial-gradient(closest-side, rgb(227 162 59 / 0.28), rgb(227 162 59 / 0.08) 62%, transparent); }
.mark { position: absolute; right: 70px; top: 95px; width: 440px; height: 440px; }
.text { position: absolute; left: 80px; top: 0; bottom: 0; width: 660px; display: flex; flex-direction: column; justify-content: center; gap: 28px; }
.lockup { height: 64px; align-self: flex-start; }
h1 { margin: 0; font-size: 58px; line-height: 1.05; letter-spacing: -0.035em; font-weight: 600; }
p { margin: 0; font-size: 24px; line-height: 1.35; color: #B3A48A; font-weight: 400; }
</style></head><body>
<div class="glow"></div>
<img class="mark" src="${data(join(brand, "mark.svg"), "image/svg+xml")}">
<div class="text">
  <img class="lockup" src="${data(join(brand, "lockup-dark.svg"), "image/svg+xml")}">
  <h1>The game engine that fits inside your app.</h1>
  <p>Rust on wgpu. Deterministic, from phones to the browser.</p>
</div>
</body></html>`;

const favicon = `<!doctype html><html><body style="margin:0;background:transparent">
<img src="${data(join(brand, "favicon.svg"), "image/svg+xml")}" width="32" height="32" style="display:block">
</body></html>`;

const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
  await page.setContent(og, { waitUntil: "load" });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: join(brand, "og.png") });
  await page.setViewportSize({ width: 32, height: 32 });
  await page.setContent(favicon, { waitUntil: "load" });
  await page.screenshot({ path: join(brand, "favicon-32.png"), omitBackground: true });
} finally {
  await browser.close();
}
