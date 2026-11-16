# The Sariema site

Next.js with Fumadocs, exported as static files. The docs pages are the
repository's `docs/*.md`, read in place; the sidebar's order is
`lib/sections.mjs`, and a doc it does not list shows under "More".

```sh
npm install        # also fetches the f.UI pieces (below)
npm run dev
npm run build      # out/: the static site; SITE_URL=https://… adds the social card
npm run check      # route types, TypeScript and scripts/*.test.mjs
```

- **The demo.** The landing plays `examples/climb` with the engine built
  for the web. `npm run engine` builds `packages/sariema-web` (it needs
  the wasm32 target and `wasm-bindgen-cli`, see `docs/web.md`); `dev` and
  `build` then copy it into `public/engine/`. Without it the landing shows
  the climb's still frame, drawn natively.
- **f.UI.** The buttons, gooey drop, theme toggle, entrances, counters,
  segmented control and cards are f.UI pieces, licensed PolyForm Strict
  1.0.0. Their CLI, `@flashz-so/f.ui`, is an exact devDependency, so the
  lockfile holds its integrity hash; at install, `scripts/fui.mjs` runs
  that local binary to fetch the pieces into `components/fui/`, which git
  ignores. They are never committed. `app/theme.css` sets their tokens to
  the Sariema palette.
- **Publishing.** `out/` carries the f.UI pieces compiled into the page.
  Whether their license allows a published site is the owner's decision,
  and nothing is published or deployed without him.
- **The brand.** `npm run brand` remakes the word, lockups and PNGs in
  `assets/brand/` from the hand-drawn marks (docs/brand.md).
- **Screenshots.** `node scripts/shots.mjs http://localhost:3000 <dir>`
  takes the landing and a docs page at 1440 and 390 px, light and dark,
  and reports any content past the right edge.
- `SARIEMA_CLI=../target/release/sariema npm run check` also builds the
  landing's Rust snippet as a game.

`scripts/brand.mjs` and `scripts/shots.mjs` drive Playwright's Chromium
(`npx playwright install chromium`).
