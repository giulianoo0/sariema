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
   start yaw. The steps begin when every role is in place. A role the game
   teleported in the same tick it played the cutscene, with a collision
   module, is judged a step later, from where the teleport landed, or from
   where it stood if the module refused it. With a collision module the
   floor holds actors at its height, so for a start within
   `cutscene.start_height` (0.25 m) above or below the actor these
   distances are measured along the ground and the actor is eased in x and
   z only, getting its collider back when the ease ends: a start a few
   centimetres off in height adds no time. A start farther off in height
   (another storey) is measured and eased to in full.
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
end, flags are set, sounds are dropped; with a collision module, a walk to
where no path leads stays where the role stood and ends with `blocked`, as
it would have played); the fade returns to where the
cutscene would have left it, and `cutscene_ended` says `skipped: true`. The
world afterwards is the one playing it through would have left.

## Steps

| Step | Does | Ends |
|---|---|---|
| `sequence: [steps]` | runs its steps in order | after the last |
| `parallel: [[steps], …]` | runs its branches together | when the longest branch ends |
| `wait: s` | | after `s` seconds |
| `wait_for: { signal \| arrived \| clip_finished \| clip_marker, timeout }` | waits for the game's `signal`, a role's arrival, a role's clip to finish, or a marker of a role's clip (`{ role, clip, marker }`); only what happens after the wait began counts | on it, or after `timeout` |
| `branch: { flag, then, else }` | runs `then` when the flag is set | after the branch taken |
| `shot: { follow \| frame \| fixed, bone \| socket, …, blend, ease }` | moves the camera: follow one role, frame two, or a fixed pose; `blend` 0 is a cut; `bone` or `socket` aims at that point of the subject (a name for `follow`, a pair with `null` for the origin for `frame`) | at once; the blend runs on |
| `walk: { role, to, speed, face }` | walks a role to a point | when it arrives, is blocked, or the game takes the walk away (`stop`, a `teleport` that lands, `move_intent`, `move_to` elsewhere ends at that target) |
| `face: { role, toward \| yaw }` | turns a role toward another or to a yaw | when it faces it |
| `look_at: { role, target }` | points a role's head at another | at once |
| `clip: { role, clip, mode, blend, speed }` | plays an animation clip at `speed` times its rate; its time follows the steps | at once |
| `hinge: { node, role, clip, span, to, ease }` | turns a hinged node of the room (`<room>.<piece>_<hinge>`) from where it is to `to` degrees from closed, over the seconds `span` of the role's clip, on that clip's time; its collider turns with it | when the clip reaches `span[1]`, or the role plays another clip |
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
| `time_scale: { scale, blend }` | plays the world at `scale` times real time (below 1 is slow motion), reached over `blend` seconds; the end of the cutscene gives back the scale it found | at once |
| `camera_shake: { amplitude, frequency, duration }` | shakes the camera, dying out: an impact | at once |
| `path: { role, clip, from, hold, mode, blend, spans, then }` | plays a clip from `from` seconds into it (still for `hold` seconds first) and moves the role's root only within its `spans` (`{ at: [t0, t1], to, yaw, rate, ease, arc }`), collider off, each span at its own `rate`; the clips of `then` follow in a row | when the last clip has played through and the root has landed |
| `scene: { set, duration, ease }` | tweens scene parameters of the active room together (`set` maps each name, like `look.exposure` or `fog.density`, to its value; `ease` is `in_out_sine` when left out); see [lighting](lighting.md#changing-the-light-and-the-look-while-playing). A skip lands it, and every scene step still under way, under the fade | at once |

Every step is also a command a game can send itself: `shot` is a rig command
then `camera_use`, `walk` is `move_to`, `face`, `look_at`, `clip`
(`play_clip`), `line` (`say`), `sound` (`play_sound`), `music`, `fade`,
`letterbox`, `set_flag`, `spawn`, `despawn`, `show`/`hide` (`set_visible`),
`teleport`, `time_scale` and `camera_shake` are their commands of the same
name, a shot's `bone` or `socket` is `rig_anchor`, `hinge` is a `tween`
of the scene parameter `hinge.<node>` that the clip's time drives instead of
the clock, `path` is `clip_path` followed by a `clip_next` for each clip
of `then`, and `scene` is `scene_tween`. The sequencer uses rig
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
does not hold, `E_CUTSCENE_COVER`. A role hidden for at least one step (`hide`,
then a block that surely takes a step: `wait` longer than 0, `wait_for` or
`line`) may teleport freely; hiding and teleporting in the same instant hides
nothing, since the viewer saw the role when that step began. At run time
nothing moves a visible actor faster than `motion.max_visible_speed` (see
[the game API](game-api.md#a-visible-actor-never-jumps)): a move that surely
would is refused with `E_TELEPORT_VISIBLE`, and an actor that still went too
far is put back. A `walk` faster than that fails the check with
`E_CUTSCENE_SPEED`, against the project's `game.settings` (or the defaults),
and so does `cutscene.start_walk_speed` in a cutscene with a role that has a
start. A `path` whose first `then` clip has an `after` earlier than the end
of the last span fails with `E_CUTSCENE_PATH_CUT`: that clip would take
over half-way and stop the path there with `blocked`. A walk goes at its speed in metres per second of the world,
whatever speed scale the game gave the role with `set_speed_scale` (that
scales the player's axis and `move_intent`, not a scripted walk), so it
takes its length over its speed, as authored. A walk the run-time
measurement stops ends there with `blocked`, whether the runtime or a
collision module moves actors.

To move an actor across the scene: walk it; or hide it, wait, teleport it,
show it; or fade out, teleport it with `"cover": "fade"`, fade in; or cut
away to another shot with `"blend": 0` and teleport it with `"cover": "cut"`
in the same instant.

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
teleport, spawn, despawn), set the same flag or tween the same scene
parameter, with a `scene` step or a `hinge` step turning its leaf
(`E_CUTSCENE_RACE`). With that, skipping lands every role, flag
and scene parameter where playing would have.

## Directing

The engine's directing rules for physical action, and what a cutscene
writes for each.

1. **Show the whole action, cause to consequence.** A traversal is never
   started mid-way: entering through a window is reaching it, opening it,
   climbing onto the sill, going through and landing. No step is skipped or
   hidden by a cut unless the cut shows the next step. Write each step as
   its own `clip`, chained by `wait_for: clip_finished`.
2. **Physical action has character.** A jump can catch a foot, stumble,
   fall and get up; the body anticipates, takes the impact and recovers
   instead of gliding. Each phase is a clip; `camera_shake` sells the
   impact.
3. **Insert shots for the beat that matters.** The key detail gets its own
   close shot, cut on the action, then back to the wide shot: a `shot` with
   `blend` 0 right after `wait_for: clip_marker` on the beat, aimed at the
   detail with `bone` or `socket`.
4. **Moving set pieces are keyed to clip time.** A hinged leaf (window,
   door) turns over a range of the actor's clip with a `hinge` step, its
   sound on a marker of the same clip; nothing opens by itself or pops
   open. Put the `hinge` step in a `parallel` with the `clip` it follows:
   it waits for the clip to play, and the leaf moves only while the clip's
   time is inside `span`, eased by `ease`. The leaf collides where it is
   drawn and stops at an actor in its way (see `docs/physics.md`, "Hinged
   leaves").
5. **Traversal in clip segments.** The root moves only within given spans
   of the clip and is still outside them (anticipation, settling); a span
   may carry its own rate. A `path` step writes it: each span names the
   seconds of the clip it covers (`at`), where the root goes (`to`, from
   where it stands when the span begins), its `ease` and `arc`, and its
   `rate` (`0.4` is slow motion for the body alone, while the world and
   the camera keep their pace). The root is placed every step from the
   clip's time, so body and root never part, in slow motion either. A
   span without `to` only changes the rate: the root stays.
6. **Intended contact is authored, not penetration.** A catching foot is a
   pose of the clip plus a point that places the root so the ankle sits on
   the edge: a span's `to` is that point. The role is posed with its
   collider off from its first span to its last, so it crosses the sill,
   and gets its collider back where the root lands, pushed out of any
   overlap; a landing above the floor falls onto it. End the last span on
   free floor (see the limits).
7. **Clip time follows the step clock.** A clip is sought to the time of
   its steps, never left to run on a clock of its own: in slow motion a
   free-running clip can freeze, and a landing waiting on it never starts.
   The runtime gives every actor's clip time: `clip_time` at the last step,
   which markers and the clip's end are judged against, and
   `clip_time_at(alpha)` to draw, interpolated between the last two steps
   like every pose. The animation module seeks to them, so a clip slows
   with `time_scale` without a still frame, and `wait_for: clip_finished`
   still comes. Leftover frame time carries into the next step, so a step
   boundary shows no still frame either. A span's rate changes the clip's
   pace from the time it is at, inside the step it gets there, never
   starting it over; the clips of a `path`'s `then` take over inside the
   step the one before ends, with the time left over, where a `wait_for:
   clip_finished` and then a `clip` lose a step or two on a held last
   frame. The one hold is a clip's first
   frame: a clip begun in a step shows frame 0 until the next step, since
   the frames drawn then have no earlier clip time to show.
8. **Insert shot on the contact, cut on the action, position continuous
   across the cut.** The close aims at the contact (`bone`), low, slowed to
   0.4× for about 0.6 s, and cuts back on the fall. A cut never hides a
   jump: the cut changes the camera only, and every role keeps moving
   continuously across it. Waits count world time: 0.6 s on screen at 0.4×
   is `wait: 0.24`.
9. **No HUD while covered.** The engine hides the game's HUD while a
   cutscene holds control and shows it again once control has returned
   and the cinema bars are gone; a node of the HUD's root in the group
   `cutscene` stays. Outside a cutscene, a
   game that covers the screen hides its HUD itself.

### Worked example: entering through a window

The hero walks to the wall, reaches and opens the window, climbs, catches a
foot on the sill, falls inside, lands and gets up. The window's leaf is a
hinged node of the room, `house.window_leaf`, that the hand pushes open
between 0.35 s and 0.9 s of `reach_open`.

```json
{
  "name": "window_entry",
  "roles": {
    "hero": { "start": { "position": [0, 0, 2], "yaw": 0 } }
  },
  "steps": [
    { "letterbox": { "to": 1, "duration": 0.4 } },
    { "shot": { "follow": "hero", "distance": 3.5, "pitch": 12, "yaw": 35, "height": 1.0, "fov": 40, "blend": 0.6 } },
    { "walk": { "role": "hero", "to": [0, 0, 0.6] } },
    { "parallel": [
      [ { "clip": { "role": "hero", "clip": "reach_open" } } ],
      [ { "hinge": { "node": "house.window_leaf", "role": "hero", "clip": "reach_open",
                     "span": [0.35, 0.9], "to": 80, "ease": "out_cubic" } } ],
      [ { "wait_for": { "clip_marker": { "role": "hero", "clip": "reach_open", "marker": "push" } } },
        { "sound": { "sound": "window_creak", "at": "hero" } } ]
    ] },
    { "wait_for": { "clip_finished": "hero" } },
    { "clip": { "role": "hero", "clip": "climb_trip", "blend": 0 } },
    { "wait_for": { "clip_marker": { "role": "hero", "clip": "climb_trip", "marker": "foot_catches" } } },
    { "shot": { "follow": "hero", "bone": "foot_l", "distance": 0.9, "pitch": 5, "yaw": 90, "height": 0, "fov": 35, "blend": 0 } },
    { "time_scale": { "scale": 0.4 } },
    { "wait": 0.24 },
    { "shot": { "follow": "hero", "distance": 3.5, "pitch": 12, "yaw": 35, "height": 1.0, "fov": 40, "blend": 0 } },
    { "time_scale": { "scale": 1, "blend": 0.15 } },
    { "wait_for": { "clip_marker": { "role": "hero", "clip": "climb_trip", "marker": "lands" } } },
    { "camera_shake": { "amplitude": 0.03, "duration": 0.25 } },
    { "wait_for": { "clip_finished": "hero" } },
    { "clip": { "role": "hero", "clip": "get_up" } },
    { "wait_for": { "clip_finished": "hero" } },
    { "letterbox": { "to": 0, "duration": 0.4 } }
  ]
}
```

| Steps | Rule |
|---|---|
| walk to the wall, `reach_open`, `climb_trip`, `get_up`, each waiting for the last | 1: the whole action, no step skipped |
| `hinge` over 0.35–0.9 s of `reach_open`, the creak on its `push` marker | 4: the leaf moves with the hand, on the clip's time |
| `climb_trip` holds the stumble and the fall; `camera_shake` on `lands` | 2: anticipation, impact, recovery |
| `wait_for: clip_marker foot_catches`, then a `shot` with `blend` 0 on `bone: foot_l` | 3 and 8: the close cuts on the beat, aimed at the contact |
| `time_scale` 0.4 for `wait: 0.24`, then back to 1 over 0.15 s with the wide shot | 8: 0.6 s of slow motion on screen |
| no `teleport`, no `hide` | 8: the cuts change the camera only |

It passes `sariema check`
(`crates/sariema-runtime/tests/fixtures/cutscenes/window_entry.json`). The
hero's root crossing the sill (5, 6) is the next example.

### Worked example: a traversal in clip spans

The hero climbs onto a sill, steps over, catches a foot, falls, lands on
its belly and gets up, in one `climb_trip` clip whose root moves only in
its airborne and climbing frames. The catch plays at 0.4 for the body
alone while a close shot cuts in on the clip's marker, and the landing
and the getting up follow with no still frame.

```json
{ "parallel": [
  [ { "path": { "role": "hero", "clip": "climb_trip", "from": 0.1, "spans": [
        { "at": [0.3, 0.6], "to": [0, 0.9, 0.15], "ease": "out_quad" },
        { "at": [0.6, 0.75], "to": [0, 0.9, -0.1] },
        { "at": [0.75, 0.9], "rate": 0.4 },
        { "at": [0.9, 1.2], "to": [0, 0, -0.8], "ease": "in_quad", "arc": 0.1 } ],
      "then": [ { "clip": "land_belly" }, { "clip": "get_up", "after": 0.6 } ] } } ],
  [ { "wait_for": { "clip_marker": { "role": "hero", "clip": "climb_trip", "marker": "foot_catches" }, "timeout": 2 } },
    { "shot": { "follow": "hero", "bone": "foot_l", "distance": 0.9, "pitch": 5, "yaw": 90, "height": 0, "fov": 35, "blend": 0 } },
    { "wait_for": { "clip_marker": { "role": "hero", "clip": "climb_trip", "marker": "falls" }, "timeout": 2 } },
    { "shot": { "follow": "hero", "distance": 4, "pitch": 12, "yaw": 35, "blend": 0 } },
    { "wait_for": { "clip_marker": { "role": "hero", "clip": "climb_trip", "marker": "lands" }, "timeout": 2 } },
    { "camera_shake": { "amplitude": 0.03, "duration": 0.25 } } ]
] }
```

| Part | Rule |
|---|---|
| `from: 0.1`, the root still until 0.3 s of the clip | 5: anticipation in place |
| spans to the sill top, across it, and down | 5 and 6: the root moves only while the body climbs or flies, collider off |
| `{ "at": [0.75, 0.9], "rate": 0.4 }` with the close shot on `foot_catches` | 3 and 8: slow motion on the contact, cut on the action |
| `then`: `land_belly`, `get_up` after 0.6 s of it | 2 and 7: impact and recovery, no still frame at the seams |

`land_belly` begins when `climb_trip` ends and `get_up` once `land_belly`
has played 0.6 s; the step ends when `get_up` has played through. It
passes `sariema check`
(`crates/sariema-runtime/tests/fixtures/cutscenes/sill_trip.json`).

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

## Limits known

- **A `hinge` step** waits for its role to play its clip for
  `cutscene.hinge_clip_wait` seconds (2); then it ends with the leaf where
  it was, and the game gets `rejected` `E_CUTSCENE_HINGE_WAIT`. A step on
  a node the active room lacks ends at once with `rejected`
  `E_SCENE_UNKNOWN`; `sariema check` and `build` refuse a node that no
  room of the project has (`E_CUTSCENE_HINGE`, naming the node and the
  step), building the rooms only when some cutscene has a `hinge` step.
  Hinges turn about a vertical axis only.
- **A path's landing** is not proven by `check`: a cutscene does not name
  the room it plays in. At run time the collider comes back where the root
  lands, pushed out of an overlap of up to the collision module's
  `max_depenetration`; past that the game hears `rejected`
  `E_TELEPORT_BLOCKED` and the root stays. Declared contacts that `check`
  proves (a foot placed on an edge by its socket) are the collision pack's
  traversals, carried by the `traverse` command.
- **A room switch inside a cutscene**: the cutscene JSON has no step for
  it; the game switches rooms under the cutscene's fade with `Cut`.
- **Bones and sockets** are placed by the animation module every step, for
  each anchor the camera lists (`Camera::anchors`); a name the actor's rig
  lacks leaves the camera on the subject's origin.
- **Clip time** is the runtime's (`State::clip_time`, and
  `State::clip_time_at` to draw): the animation module must seek every clip
  to it and report `clip_marker` and `clip_finished` from it; a module that
  runs clips on its own clock breaks rule 7.
- **The HUD** has no step: the game hides and shows it (rule 9).
- **One cutscene at a time**; a second `cutscene_play` is refused with
  `E_CUTSCENE_BUSY`. Skipping drops sounds and shakes.

## What we took from other engines

- Clip markers a cutscene waits on: Unreal's Anim Notifies and Unity's
  Animation Events and Timeline signals, read here as happenings the
  sequencer waits for instead of callbacks.
- Bones and sockets as camera targets: Unreal's skeletal mesh sockets and
  Godot's `BoneAttachment3D`, kept as names until the animation module
  resolves them.
- A traversal as root motion keyed to spans of its clip: Unreal's root
  motion from montages, which also moves the capsule only while the
  animation does; here each span is an eased line to a point instead of a
  curve extracted from the clip, so a clip without root motion drives it.
- Clips queued in a row with the time left over: Godot's
  `AnimationNodeStateMachine` "at end" transitions and Unity's exit time,
  taken at the exact time inside the step instead of on the next frame.
- Slow motion as a scale on the clock that feeds the fixed step, as Bevy's
  `Time<Virtual>` relative speed does, instead of a smaller step: a step
  stays 1/60 s of world time, so play stays deterministic, and clips that
  follow the steps slow with everything else.
- Camera blends from the pose on screen and impulse shakes: Unity's
  Cinemachine.
- Shots, cuts and blends as the steps of one timeline, as in Unreal's
  Sequencer camera cuts track and Unity's Timeline.
