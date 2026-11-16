# Cutscenes

A cutscene is a JSON file (`schemas/cutscene.schema.json`) in the project's
cutscene directory: the roles it casts, where each one starts, and the steps
the engine plays. `sariema build` validates it against the schema, compiles
it into the game pack and checks its rules; `sariema check` runs the same
checks alone. The engine's sequencer plays it natively, step by step, with
the same commands a game could send.

```json
{
  "name": "meet",
  "roles": {
    "hero": { "start": { "position": [0, 0, -5.5], "yaw": 0 } },
    "friend": { "start": { "position": [0, 0, -8], "yaw": 180 } }
  },
  "steps": [
    { "letterbox": { "to": 1, "duration": 0.4 } },
    { "shot": { "frame": ["hero", "friend"], "pitch": 25, "blend": 0.8 } },
    { "walk": { "role": "hero", "to": [0, 0, -6.8], "speed": 1.2 } },
    { "shot": { "follow": "friend", "distance": 3, "pitch": 12, "yaw": 180, "blend": 0.6 } },
    { "line": { "speaker": "friend", "text": "Hello! You found the meadow.", "advance": "auto", "duration": 2.5 } },
    { "set_flag": "met" },
    { "letterbox": { "to": 0, "duration": 0.4 } }
  ]
}
```

Times are seconds, angles degrees (a yaw of 0 faces −Z and grows
counter-clockwise seen from above), distances metres.

## Playing one

The game binds an entity to every role, then plays the cutscene by name:

```rust
ctx.bind(id("hero"), self.hero);
ctx.bind(id("friend"), self.friend);
ctx.play_cutscene(id("meet"));
```

A play is refused, with a `rejected` event, when a cutscene is already
playing (`E_CUTSCENE_BUSY`), the pack has no cutscene of that name
(`E_CUTSCENE_UNKNOWN`), or a role is not bound as it must be
(`E_CUTSCENE_UNBOUND`): a role with a start place needs a live entity, a
role the cutscene spawns needs a free handle (`ctx.reserve()`).

Then, in order:

1. The hero is handed to the cutscene (`control` event) and
   `cutscene_started` is sent.
2. **Placement.** Every role with a start place is brought there without a
   jump: a hidden actor is placed; one within `cutscene.start_tolerance`
   (0.02 m) only turns; one within `cutscene.start_blend_max` (0.3 m) is
   eased there over `cutscene.start_blend` (0.25 s); a farther one walks
   there at `cutscene.start_walk_speed` (1.5 m/s). Each then turns to its
   start yaw. The steps begin when every role is in place.
3. **The steps** run in order. A step that takes time (a walk, a line, a
   wait, a blend) holds the next one until it ends.
4. **The end.** The hero goes back to whoever had it, where the cutscene
   left it; the input buffer is cleared, so no press made during the
   cutscene acts afterwards; the camera blends back to the rig the game was
   using over `cutscene.end_blend` (0.6 s); `cutscene_ended` is sent.

**Skipping.** The game sends `cutscene_skip`, or the player presses
`cutscene.skip_action` (`skip`) during a skippable cutscene. The screen fades
out over `cutscene.skip_fade` (0.3 s); under the fade every remaining step
reaches its end state at once (walks land on their targets, lines and waits
end, flags are set, sounds are dropped); the fade returns to where the
cutscene would have left it, and `cutscene_ended` says `skipped: true`. The
world afterwards is the one playing it through would have left.

## Steps

| Step | Does | Ends |
|---|---|---|
| `sequence: [steps]` | runs its steps in order | after the last |
| `parallel: [[steps], …]` | runs its branches together | when the longest branch ends |
| `wait: s` | | after `s` seconds |
| `wait_for: { signal \| arrived \| clip_finished, timeout }` | waits for the game's `signal`, a role's arrival, or a role's clip; only what happens after the wait began counts | on it, or after `timeout` |
| `branch: { flag, then, else }` | runs `then` when the flag is set | after the branch taken |
| `shot: { follow \| frame \| fixed, …, blend, ease }` | moves the camera: follow one role, frame two, or a fixed pose; `blend` 0 is a cut | at once; the blend runs on |
| `walk: { role, to, speed, face }` | walks a role to a point | when it arrives |
| `face: { role, toward \| yaw }` | turns a role toward another or to a yaw | when it faces it |
| `look_at: { role, target }` | points a role's head at another | at once |
| `clip: { role, clip, mode, blend }` | plays an animation clip | at once |
| `line: { speaker, text, voice, advance, duration }` | a dialogue line; `auto` ends after `duration`, `tap` on `cutscene.advance_action` | when the line ends |
| `sound: { sound, bus, at, volume }` | plays a sound, at a role when `at` names one | at once |
| `music: { music, fade }` | crossfades to a track, or stops the music without `music` | at once |
| `fade: { to, duration, color }` | fades the screen; 1 is fully covered | after `duration` |
| `letterbox: { to, duration }` | cinema bars | after `duration` |
| `set_flag` / `clear_flag` | changes a flag; the game gets `flag_changed` | at once |
| `spawn: { role, position, yaw }` | creates a role that has no start place | at once |
| `despawn` / `show` / `hide` | removes, shows or hides a role | at once |
| `teleport: { role, position, yaw, cover }` | moves a role at once, under a declared cover | at once |
| `call: name` | sends the game `cutscene_call` | at once |

Every step is also a command a game can send itself: `shot` is a rig command
then `camera_use`, `walk` is `move_to`, `face`, `look_at`, `clip`
(`play_clip`), `line` (`say`), `sound` (`play_sound`), `music`, `fade`,
`letterbox`, `set_flag`, `spawn`, `despawn`, `show`/`hide` (`set_visible`)
and `teleport` are their commands of the same name. The sequencer uses rig
id and line id `u32::MAX` and tween ids from `0xFFFF0000` up; a game leaves
those to it.

## The rules

`sariema build` and `sariema check` refuse a cutscene that breaks one of
these rules, naming the file, the cutscene and the step, like
`cutscenes/meet.json: cutscene meet, steps[4].teleport: …`. The sequencer and
the engine enforce the same rules while playing.

### No actor jumps on screen

An actor that may be visible only moves continuously: it walks, turns or is
eased. A `teleport` of a role that may be on screen must declare what hides
the jump, and the check proves the cover holds:

- `"cover": "fade"`: the screen is known to be fully faded (a `fade` to 1
  before it, on every path to it).
- `"cover": "cut"`: a `shot` with `blend` 0 comes earlier in the same
  instant, with nothing that takes time in between.

Without a cover the check fails with `E_CUTSCENE_TELEPORT`; with a cover that
does not hold, `E_CUTSCENE_COVER`. A hidden role (`hide` first) may teleport
freely. At run time a `teleport` or `set_transform` of a visible actor that
is not covered is refused with `E_TELEPORT_VISIBLE`.

To move an actor across the scene: walk it; or hide it, teleport it, show it;
or fade out, teleport it with `"cover": "fade"`, fade in; or cut away to
another shot with `"blend": 0` and teleport it with `"cover": "cut"` in the
same instant.

### Every role starts somewhere

Every role declares `start` (where it must be when the steps begin), unless
the cutscene spawns it before using it. A role used without either fails with
`E_CUTSCENE_START`. Using a role after its `despawn` fails with
`E_CUTSCENE_ABSENT`; spawning a role that has a start place, or that may
already be in the world, fails with `E_CUTSCENE_SPAWN`. Because every role
starts from a declared place, the sequencer can bring it there without a
jump, wherever the game left it.

### Parallel branches do not race

Two branches of one `parallel` never move the same role (walk, face,
teleport, spawn, despawn) or set the same flag (`E_CUTSCENE_RACE`). With
that, skipping lands every role and flag where playing would have.

## Writing one that passes

1. Give every role a `start` that matches where the game usually has the
   actor; the sequencer walks it there otherwise, in view.
2. Put a `shot` with a `blend` before the first line, so the camera eases in.
3. Move roles with `walk` and `face`; reserve `teleport` for cuts and fades.
4. End with the cinema bars and any fade back at 0; the camera blends back
   to the game's rig by itself.
5. Run `sariema check <project> --json`, then
   `sariema run --headless --game <pack> --input <log> --steps N --json` and
   read `cutscene_started`, `cutscene_ended` and the steps between.
