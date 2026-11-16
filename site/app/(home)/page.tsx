// SPDX-License-Identifier: Apache-2.0

import Link from "next/link";
import { highlight } from "fumadocs-core/highlight";
import { CodeBlock, Pre } from "fumadocs-ui/components/codeblock";
import { BlurFade } from "@/components/fui/blur-fade";
import { NumberTicker } from "@/components/fui/number-ticker";
import { DotPattern } from "@/components/fui/pattern";
import { SpotlightCard } from "@/components/fui/spotlight-card";
import button from "@/components/fui/button.module.css";
import { CodeTabs } from "@/components/landing/code-tabs";
import { Demo } from "@/components/landing/demo";
import { GooeyLink } from "@/components/landing/gooey-link";
import { repo } from "@/lib/markdown.mjs";
import { reactNative, rust, typescript } from "./snippets";
import styles from "./landing.module.css";

const stats = [
  { value: 0.88, decimals: 2, unit: "ms", label: "CPU for one step of examples/climb and its frame at 1920×1080" },
  { value: 1.21, decimals: 2, unit: "ms", label: "GPU for that frame: room, actors, HUD and post-processing" },
  { value: 2000, decimals: 0, unit: "", label: "actors walking under collision, each looping a clip, in 1.36 ms a step" },
  { value: 16, from: 23, decimals: 0, unit: "", label: "render passes for the full look, down from 23; a frame's CPU follows the pass count" },
];

const features = [
  { title: "A step that repeats", href: "/docs/engine/", body: "A fixed 1/60 s step. The same input log gives the same engine_hash natively and in every browser the tests run." },
  { title: "Games as data", href: "/docs/game-api/", body: "A game is packs and WebAssembly logic, run under a fuel budget and a memory ceiling. No native code is ever downloaded." },
  { title: "A renderer on wgpu", href: "/docs/performance/", body: "Clustered forward lights, a shadow atlas, ambient occlusion and bloom, with a downlevel path for WebGL2." },
  { title: "Movement and collision", href: "/docs/physics/", body: "Deterministic character movement, collision packs, and sariema check for spots and routes." },
  { title: "Actors and cutscenes", href: "/docs/cutscenes/", body: "Skeletal clips, crossfades and events; cutscenes with rules against teleports, checked before they ship." },
  { title: "Sound and UI", href: "/docs/audio/", body: "A mixer with buses, streamed music and ducking; an in-engine flexbox UI with distance-field text and touch widgets." },
];

const platforms = [
  { name: "Desktop", detail: "The sariema CLI and its window player, natively through wgpu.", status: "Works today" },
  { name: "Web", detail: "WebGPU, or WebGL2 through the downlevel path, with the native engine_hash.", status: "Works today" },
  { name: "App WebViews", detail: "WKWebView and Android's System WebView run the same web package.", status: "Works today" },
  { name: "React Native", detail: "Inside react-native-webview now; a native Fabric component is the design's goal.", status: "Planned natively" },
];

async function code(source: string, lang: string) {
  return highlight(source, {
    lang,
    themes: { light: "solarized-light", dark: "gruvbox-dark-medium" },
    defaultColor: false,
    components: {
      pre: (props) => (
        <CodeBlock>
          <Pre {...props} />
        </CodeBlock>
      ),
    },
  });
}

export default async function Home() {
  const tabs = [
    { value: "rust", label: "Rust", caption: "A game's logic, with sariema-sdk.", code: await code(rust, "rust") },
    { value: "ts", label: "TypeScript", caption: "Play a game on a canvas with @sariema/web.", code: await code(typescript, "ts") },
    { value: "rn", label: "React Native", caption: "The web package inside an app's WebView.", code: await code(reactNative, "tsx") },
  ];
  return (
    <main className={styles.main}>
      <section className={styles.hero}>
        <div className={styles.sun} aria-hidden />
        <DotPattern className={styles.dots} gap={18} />
        <BlurFade index={0} inView={false} className={styles.eyebrow}>
          <span>Open source · Apache-2.0</span>
        </BlurFade>
        <BlurFade index={1} inView={false}>
          <h1 className={styles.title}>The game engine that fits inside your app.</h1>
        </BlurFade>
        <BlurFade index={2} inView={false}>
          <p className={styles.lead}>
            Sariema is written in Rust on wgpu. Games ship as data packs with sandboxed WebAssembly logic, and step to
            the same hash on desktop and in the browser.
          </p>
        </BlurFade>
        <BlurFade index={3} inView={false} className={styles.actions}>
          <GooeyLink href="/docs/">Read the docs</GooeyLink>
          <a className={button.button} data-variant="secondary" data-size="lg" href={repo}>
            View on GitHub
          </a>
        </BlurFade>
        <div className={styles.demo}>
          <Demo />
        </div>
      </section>

      <section className={styles.section} aria-labelledby="numbers">
        <BlurFade>
          <h2 id="numbers" className={styles.heading}>
            Measured, not promised
          </h2>
          <p className={styles.sub}>
            Apple M5, Metal, medium tier, release builds; medians of three repetitions on a shared machine. From{" "}
            <Link href="/docs/performance/">the performance notes</Link>.
          </p>
        </BlurFade>
        <dl className={styles.stats}>
          {stats.map((s, i) => (
            <BlurFade key={s.label} index={i} className={styles.stat}>
              <dt>{s.label}</dt>
              <dd>
                <NumberTicker value={s.value} from={s.from} decimals={s.decimals} locale="en-US" />
                {s.unit && <span className={styles.unit}>{s.unit}</span>}
              </dd>
            </BlurFade>
          ))}
        </dl>
      </section>

      <section className={styles.section} aria-labelledby="features">
        <BlurFade>
          <h2 id="features" className={styles.heading}>
            One engine, every module wired
          </h2>
        </BlurFade>
        <div className={styles.features}>
          {features.map((f, i) => (
            <BlurFade key={f.title} index={i}>
              <Link href={f.href} className={styles.featureLink}>
                <SpotlightCard className={styles.feature}>
                  <h3>{f.title}</h3>
                  <p>{f.body}</p>
                </SpotlightCard>
              </Link>
            </BlurFade>
          ))}
        </div>
      </section>

      <section className={styles.section} aria-labelledby="code">
        <BlurFade>
          <h2 id="code" className={styles.heading}>
            Write the game, then put it anywhere
          </h2>
        </BlurFade>
        <BlurFade index={1}>
          <CodeTabs tabs={tabs} />
        </BlurFade>
      </section>

      <section className={styles.section} aria-labelledby="platforms">
        <BlurFade>
          <h2 id="platforms" className={styles.heading}>
            Where it runs
          </h2>
        </BlurFade>
        <div className={styles.platforms}>
          {platforms.map((p, i) => (
            <BlurFade key={p.name} index={i} className={styles.platform}>
              <span className={styles.status} data-planned={p.status !== "Works today" || undefined}>
                {p.status}
              </span>
              <h3>{p.name}</h3>
              <p>{p.detail}</p>
            </BlurFade>
          ))}
        </div>
      </section>

      <footer className={styles.footer}>
        <img src="/brand/lockup.svg" alt="Sariema" className={styles.lockupLight} height={28} />
        <img src="/brand/lockup-dark.svg" alt="Sariema" className={styles.lockupDark} height={28} />
        <nav className={styles.footerLinks}>
          <Link href="/docs/">Docs</Link>
          <Link href="/docs/brand/">Brand</Link>
          <a href={repo}>GitHub</a>
          <span>Apache-2.0</span>
        </nav>
      </footer>
    </main>
  );
}
