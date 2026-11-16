// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { heading, rewriteLink, summary } from "../lib/markdown.mjs";
import { sections } from "../lib/sections.mjs";

const docs = new URL("../../docs/", import.meta.url);
const files = readdirSync(docs).filter((f) => f.endsWith(".md"));
const read = (file) => readFileSync(new URL(file, docs), "utf8");

test("every doc has a title and a summary for search and meta tags", () => {
  for (const file of files) {
    assert.ok(heading(read(file)), `${file} has no # title`);
    assert.ok(summary(read(file)).length > 40, `${file} has no opening paragraph`);
  }
});

test("the sidebar lists docs that exist, each once", () => {
  const listed = sections.flatMap((s) => s.docs.map(([slug]) => slug));
  assert.equal(new Set(listed).size, listed.length);
  for (const slug of listed) assert.ok(files.includes(`${slug}.md`), `docs/${slug}.md is missing`);
});

test("a link to another doc becomes its page; any other relative link goes to GitHub", () => {
  assert.equal(rewriteLink("web.md#numbers"), "/docs/web/#numbers");
  assert.equal(rewriteLink("./engine.md"), "/docs/engine/");
  assert.equal(rewriteLink("#textures"), "#textures");
  assert.equal(rewriteLink("https://wgpu.rs"), "https://wgpu.rs");
  assert.equal(rewriteLink("../examples/climb"), "https://github.com/giulianoo0/sariema/blob/main/examples/climb");
});

test("every link between docs points at a doc that exists", () => {
  for (const file of files) {
    for (const [, target] of read(file).matchAll(/\]\(([\w.-]+\.md)(#[^)]*)?\)/g)) {
      assert.ok(existsSync(new URL(target, docs)), `${file} links to ${target}`);
    }
  }
});

test("a summary is plain text and stops at a word", () => {
  const text = summary("# T\n\nSee [the `web` page](web.md) for **more**.\n\nNext.");
  assert.equal(text, "See the web page for more.");
  assert.ok(summary(`# T\n\n${"word ".repeat(80)}`).endsWith("word…"));
});
