// SPDX-License-Identifier: Apache-2.0

// f.UI is licensed PolyForm Strict 1.0.0: its pieces are fetched by the
// public CLI into components/fui/ (git-ignored) and never committed.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const cli = "@flashz-so/f.ui@0.2.0";
const pieces = [
  "botao",
  "gooey",
  "troca-de-tema",
  "aparecer-com-desfoque",
  "numero-que-conta",
  "segmentado",
  "cartao-de-luz",
  "fundo-de-pontos",
];

const stamp = join(site, "components", "fui", ".pieces");
const want = `${cli} ${pieces.join(" ")}\n`;
if (existsSync(stamp) && readFileSync(stamp, "utf8") === want) process.exit(0);

execFileSync("npx", ["-y", cli, "add", ...pieces, "-y", "--no-docs"], { cwd: site, stdio: "inherit" });
writeFileSync(stamp, want);
