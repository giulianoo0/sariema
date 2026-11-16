// SPDX-License-Identifier: Apache-2.0

// The Rust snippet is checked by scripts/snippets.test.mjs, which builds it
// as a game with the CLI when SARIEMA_CLI names one.

export const rust = `use sariema_sdk::{Command, Ctx, Entity, Event, Events, Game, Kind, game, id};

const NEAR_FRIEND: u32 = 1;
const MEET: u32 = id("meet");

pub struct Hello {
    hero: Entity,
}

impl Game for Hello {
    fn new(ctx: &mut Ctx<'_>) -> Hello {
        let hero = ctx.spawn(Kind::Actor, id("hero"));
        ctx.send(Command::SetHero { entity: hero, speed: 2.0 });
        ctx.send(Command::Zone { zone: NEAR_FRIEND, center: [0.0, 0.0, -8.0], radius: 3.0 });
        Hello { hero }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, events: Events<'_>) {
        for event in events {
            if let Event::TriggerEnter { trigger: NEAR_FRIEND, entity } = event {
                if entity == self.hero {
                    ctx.bind(id("hero"), self.hero);
                    ctx.play_cutscene(MEET);
                }
            }
        }
    }
}

game!(Hello);
`;

export const typescript = `import { mount } from "@sariema/web";

const canvas = document.querySelector("canvas")!;
const sariema = await mount(canvas, { quality: "medium" });

// A folder written by \`sariema web\`: the game pack and its manifest.
await sariema.loadGame("/games/climb/");

sariema.on("host", (message) => console.log(message.type, message.step));
sariema.on("error", (error) => console.error(error.code));
`;

export const reactNative = `import { WebView } from "react-native-webview";

// The page at this address mounts @sariema/web and forwards events with
// window.ReactNativeWebView.postMessage(JSON.stringify(event)).
export function GameScreen() {
  return (
    <WebView
      source={{ uri: "https://example.com/games/climb/" }}
      allowsInlineMediaPlayback
      mediaPlaybackRequiresUserAction={false}
      onMessage={(e) => console.log(JSON.parse(e.nativeEvent.data))}
    />
  );
}
`;
