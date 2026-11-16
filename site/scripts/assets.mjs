// SPDX-License-Identifier: Apache-2.0

// Copies into public/ what the site serves from elsewhere in the repository:
// the brand files from assets/brand/ and, when packages/sariema-web has been
// built (`npm run engine`), the engine and the climb game the landing plays.
// Without that build the landing shows the climb's still frame.

import { cpSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const repo = join(site, "..");
const pub = join(site, "public");

rmSync(join(pub, "brand"), { recursive: true, force: true });
cpSync(join(repo, "assets", "brand"), join(pub, "brand"), { recursive: true });

const dist = join(repo, "packages", "sariema-web", "dist");
const engine = join(pub, "engine");
rmSync(engine, { recursive: true, force: true });
if (!existsSync(join(dist, "index.js"))) {
  console.log("assets: no packages/sariema-web/dist; the landing will show the still frame (npm run engine builds it)");
  process.exit(0);
}
mkdirSync(join(engine, "wasm"), { recursive: true });
for (const file of ["index.js", "audio.js", "fetch.js", "audio-worklet.js", "wasm/sariema_web.js", "wasm/sariema_web_bg.wasm"]) {
  cpSync(join(dist, file), join(engine, file));
}
cpSync(join(dist, "demo"), join(engine, "demo"), { recursive: true });
console.log("assets: engine and climb copied to public/engine");
