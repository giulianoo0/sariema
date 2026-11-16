// SPDX-License-Identifier: Apache-2.0

//! The single source of truth for every command a game sends and every
//! event it receives. The decoder, the SDK's encoder and `sariema api` are
//! all generated from this table; shared verbatim with `sariema-sdk`.
//!
//! Angles are degrees, distances metres, durations seconds. A yaw of 0 faces
//! −Z (north) and grows counter-clockwise seen from above. Ids named after a
//! thing (`cutscene`, `flag`, `clip`, `sound`, `role`, `signal`, `call`) are
//! [`name_id`](super::wire::name_id) of its name; `tween`, `timer`, `rig`,
//! `zone`, `line` and `var` ids are chosen by the game.

use super::wire::{
    Advance, Anchor, Bus, Bytes, ClipMode, Controller, Ease, Entity, EnumSpec, FieldSpec, Kind,
    MessageSpec, Orientation, Property, Reader, RoomTransition, Sink, Str, Value, Vec3, Wire,
    WireError,
};

batch_table! {
    version 1;
    enums [Kind, Ease, Property, ClipMode, Bus, Controller, Advance, Orientation, Anchor, RoomTransition]
    commands {
        /// Brings an entity to life in a free slot the game chose. The slot's
        /// generation must be newer than any it had before.
        0x0001 Spawn spawn { entity: Entity, kind: Kind, asset: u32 }
        /// Removes an entity, its children's parent link, its tweens and followers.
        0x0002 Despawn despawn { entity: Entity }
        /// Places an entity. For a visible actor this is a teleport and the
        /// same rule applies: refused unless covered (see `teleport`).
        0x0003 SetTransform set_transform { entity: Entity, position: Vec3, yaw: f32 }
        /// `parent` NONE detaches. Positions become relative to the parent.
        0x0004 SetParent set_parent { entity: Entity, parent: Entity }
        0x0005 SetVisible set_visible { entity: Entity, visible: bool }
        0x0006 SetScale set_scale { entity: Entity, scale: f32 }
        0x0007 SetTag set_tag { entity: Entity, tag: u32, on: bool }

        /// Makes `entity` the hero the player's move axis drives at `speed` m/s.
        0x0010 SetHero set_hero { entity: Entity, speed: f32 }
        /// Hands the hero over. Clears the input action buffer.
        0x0011 Control control { by: Controller }
        /// Walks at `speed` m/s towards `target`; ends with `arrived` or `blocked`.
        0x0012 MoveTo move_to { entity: Entity, target: Vec3, speed: f32 }
        /// A velocity to hold until changed, in m/s, horizontal.
        0x0013 MoveIntent move_intent { entity: Entity, velocity: Vec3 }
        0x0014 Stop stop { entity: Entity }
        /// Moves at once. A visible actor may only teleport while the screen is
        /// fully faded, in the step of a camera cut, or with `force`; otherwise
        /// the command is rejected with E_TELEPORT_VISIBLE.
        0x0015 Teleport teleport { entity: Entity, position: Vec3, yaw: f32, force: bool }
        /// Turns at the configured turn rate until facing `yaw`.
        0x0016 Face face { entity: Entity, yaw: f32 }
        /// Head and eyes follow `target`; NONE stops.
        0x0017 LookAt look_at { entity: Entity, target: Entity }
        0x0018 SetSpeedScale set_speed_scale { entity: Entity, scale: f32 }
        /// An impulse in m/s for the collision module.
        0x0019 Knockback knockback { entity: Entity, impulse: Vec3 }
        0x001A SetCollider set_collider { entity: Entity, enabled: bool }
        /// Carries an actor along the room's traversal `traversal` (a name in
        /// its collision pack) over `duration` seconds, collider off, its
        /// declared contacts touched by the actor's socket or joint of the
        /// same name; ends with `arrived` on free floor. Needs a collision
        /// module: with `motion.builtin` it ends at once with `blocked`.
        0x001B Traverse traverse { entity: Entity, traversal: u32, duration: f32 }

        /// `blend` is the crossfade from the current clip, in seconds.
        0x0020 PlayClip play_clip { entity: Entity, clip: u32, mode: ClipMode, blend: f32, speed: f32 }

        /// `emitter` NONE plays without position. `volume` is linear gain.
        0x0030 PlaySound play_sound { sound: u32, bus: Bus, emitter: Entity, volume: f32 }
        /// Crossfades to `music` over `fade` seconds.
        0x0031 PlayMusic play_music { music: u32, fade: f32 }
        0x0032 StopMusic stop_music { fade: f32 }
        0x0033 SetBusVolume set_bus_volume { bus: Bus, volume: f32, fade: f32 }

        0x0040 UiVisible ui_visible { node: u32, visible: bool }
        0x0041 UiText ui_text { node: u32, text: Str }
        0x0042 UiValue ui_value { node: u32, value: f32 }
        /// Shows a dialogue line; `line_done` reports its end. `voice` 0 is
        /// silent. `duration` is used by `Auto` and ignored by `Tap`.
        0x0043 Say say { line: u32, speaker: Entity, text: Str, voice: u32, advance: Advance, duration: f32 }
        /// Ends the current `Tap` line.
        0x0044 AdvanceLine advance_line {}
        /// Plays the HUD animation named `animation`.
        0x0045 UiPlay ui_play { animation: u32 }

        /// Fades the screen to `to` (0 clear, 1 covered) with `color`, linear RGB.
        0x0050 Fade fade { to: f32, duration: f32, color: Vec3 }
        /// Cinema bars, 0 none to 1 full.
        0x0051 Letterbox letterbox { to: f32, duration: f32 }
        /// Plays the world at `scale` times real time, reached over `blend`
        /// seconds of world time; below 1 is slow motion. A step stays 1/60 s
        /// of world time, so play stays deterministic: only how many steps
        /// run per real second changes. Kept within `min_time_scale` and
        /// `max_time_scale`.
        0x0052 TimeScale time_scale { scale: f32, blend: f32 }

        /// Animates a property from its current value. A new tween or spring on
        /// the same property replaces the old one without a jump.
        0x0060 Tween tween { tween: u32, entity: Entity, property: Property, var: u32, to: f32, duration: f32, ease: Ease }
        /// A damped spring towards `to`: `frequency` in Hz, `damping` 1 is critical.
        0x0061 Spring spring { tween: u32, entity: Entity, property: Property, var: u32, to: f32, frequency: f32, damping: f32 }
        0x0062 CancelTween cancel_tween { tween: u32 }
        0x0063 SetVar set_var { var: u32, value: f32 }
        /// Keeps `entity` at `target`'s position plus `offset`, catching up with
        /// a half-life of `lag` seconds; 0 is rigid.
        0x0064 Follow follow { entity: Entity, target: Entity, offset: Vec3, lag: f32 }
        0x0065 Unfollow unfollow { entity: Entity }
        /// Fires `timer` after `duration` seconds, again every `duration` if `repeat`.
        0x0066 Timer timer { timer: u32, duration: f32, repeat: bool }
        0x0067 CancelTimer cancel_timer { timer: u32 }
        /// A sphere that reports actors entering and leaving it.
        0x0068 Zone zone { zone: u32, center: Vec3, radius: f32 }
        0x0069 RemoveZone remove_zone { zone: u32 }

        /// A camera rig that follows `subject` from `distance` m at `pitch`
        /// and `yaw`, aimed `height` m above it.
        0x0070 RigFollow rig_follow { rig: u32, subject: Entity, distance: f32, pitch: f32, yaw: f32, height: f32, fov: f32 }
        /// A rig that keeps both subjects in frame with `margin` m around them.
        0x0071 RigFrame rig_frame { rig: u32, a: Entity, b: Entity, pitch: f32, yaw: f32, height: f32, fov: f32, margin: f32 }
        0x0072 RigFixed rig_fixed { rig: u32, position: Vec3, look_at: Vec3, fov: f32 }
        /// Follow tuning: the subject moves freely inside `dead_zone` m; the
        /// focus springs after it at `frequency` Hz with `damping`, leading by
        /// the subject's velocity times `look_ahead` seconds.
        0x0073 RigTuning rig_tuning { rig: u32, dead_zone: f32, frequency: f32, damping: f32, look_ahead: f32 }
        /// The box the rig's focus stays inside.
        0x0074 RigBounds rig_bounds { rig: u32, min: Vec3, max: Vec3 }
        /// Switches to `rig`, blending over `blend` seconds; 0 is a cut, the
        /// only way the camera moves discontinuously.
        0x0075 CameraUse camera_use { rig: u32, blend: f32, ease: Ease }
        0x0076 CameraShake camera_shake { amplitude: f32, frequency: f32, duration: f32 }
        /// Aims `rig` at a bone or socket of `subject`, while `subject` is in
        /// its framing, instead of at its origin; `origin` aims at the origin
        /// again. The animation module resolves `name` to a place.
        0x0077 RigAnchor rig_anchor { rig: u32, subject: Entity, anchor: Anchor, name: u32 }

        /// Binds a cutscene role to an entity for the next `cutscene_play`.
        0x0080 CutsceneBind cutscene_bind { role: u32, entity: Entity }
        0x0081 CutscenePlay cutscene_play { cutscene: u32 }
        /// Jumps to the end state under a fade.
        0x0082 CutsceneSkip cutscene_skip {}
        /// Releases a cutscene waiting on `signal`.
        0x0083 Signal signal { signal: u32 }
        0x0084 SetFlag set_flag { flag: u32, on: bool }

        /// Forgets a buffered press of `action`.
        0x0090 Consume consume { action: u16 }

        /// Starts loading room `room` (its name id) of the pack in the
        /// background, so a later `room_switch` to it shows no wait. A host
        /// keeps at most its setting of rooms loaded, dropping the ones used
        /// longest ago.
        0x00B0 RoomPreload room_preload { room: u32 }
        /// Makes `room` the active room: its collision moves the actors, its
        /// lighting lights the scene, its groups, nodes, pieces and scene
        /// parameters are the ones the next commands address. Actors, sound
        /// and the camera go on as they were. `duration` is how long a
        /// continuous switch or a dissolve takes; a cut takes none. A room
        /// not loaded yet is loaded at once, which can stall that frame.
        0x00B1 RoomSwitch room_switch { room: u32, transition: RoomTransition, duration: f32 }
        /// Switches a group of the active room, its draws and lights, on or off.
        0x00B2 RoomGroup room_group { group: u32, on: bool }
        /// Shows `state` of a piece of the active room; state 0 is the look
        /// before any state.
        0x00B3 PieceState piece_state { piece: u32, state: u32 }

        0x00A0 HostReady host_ready {}
        0x00A1 HostExit host_exit {}
        0x00A2 HostOrientation host_orientation { orientation: Orientation }
        /// Asks the host to persist `value` under `key`.
        0x00A3 Save save { key: Str, value: Bytes }
        /// Asks the host for every saved key; answered by `save_loaded` events
        /// and one `save_load_done`.
        0x00A4 SaveLoad save_load {}
        /// A message on a topic the host and game agreed on.
        0x00A5 HostSend host_send { topic: Str, payload: Bytes }

        /// Plays `clip` on `entity` from `from` seconds into it, still for
        /// `hold` seconds first, and moves the actor's root only within the
        /// clip's `spans` (`ClipSpan::put` writes them, in order of the
        /// clip's time, none overlapping): within a span the clip plays at
        /// the span's rate and the root goes from where it stands when the
        /// span begins to the span's point, eased, collider off; outside
        /// them the root stays and the clip plays at its own rate. Ends with
        /// `arrived` when the last span ends, the collider on again where
        /// the root stands, pushed out of any overlap. Refused with
        /// E_CLIP_SPANS for spans out of order, past the clip's end or not
        /// finite, and with E_TELEPORT_VISIBLE when a span would carry a
        /// visible actor faster than it may be seen to move.
        0x0021 ClipPath clip_path { entity: Entity, clip: u32, mode: ClipMode, blend: f32, from: f32, hold: f32, spans: Bytes }
        /// Queues `clip` after the clip `entity` plays and those queued
        /// after it: it begins once the one before has played `after`
        /// seconds, or reached its end when `after` is 0, with the time left
        /// over, so the seam shows no still frame. Plays at once when the
        /// actor plays no clip. A `play_clip` or `clip_path` drops the queue.
        0x0023 ClipNext clip_next { entity: Entity, clip: u32, mode: ClipMode, blend: f32, speed: f32, after: f32 }
        /// Plays the clip `entity` plays at `rate` from this step, going on
        /// from where it is: no restart and no crossfade, so a walk cycle
        /// follows the actor's pace without its feet sliding. 0 holds it
        /// where it is. Within a `clip_path` span, the next span's rate
        /// replaces it when that span begins; the clips queued after it keep
        /// their own. Refused with E_CLIP_NOT_PLAYING when the actor plays
        /// another clip or none, and with E_CLIP_RATE for a rate below 0 or
        /// not finite.
        0x0022 ClipRate clip_rate { entity: Entity, clip: u32, rate: f32 }
        /// `actor` holds `entity` (a room node, a light, an empty) at its
        /// socket or joint `socket`, a socket looked up first: the entity
        /// stays where it stands on the socket when `keep`, else sits at
        /// `offset` from it, in the socket's frame and turned with it. Either
        /// way it is placed each step on the socket rigidly, in metres of the
        /// world whatever the actor's scale, and taken out of any parent,
        /// follow or tween. `actor` NONE lets go: the entity stays where it
        /// was. Refused with E_HOLD_ACTOR for an actor to hold, and with
        /// E_SOCKET_UNKNOWN, a step later, for a socket or joint the actor's
        /// model lacks; an actor drawn with no model keeps it where it is.
        0x00E0 Hold hold { entity: Entity, actor: Entity, socket: u32, offset: Vec3, keep: bool }
        /// Points the spot light `entity` (a room light spawned as an
        /// entity) at `target` in `at`'s frame, in metres of the world
        /// whatever `at`'s scale, following `at` as it moves and turns: `at`
        /// the light itself, held in a hand, lights where the hand points.
        /// `at` NONE aims at a point of the world. A point light has no aim.
        /// Despawning `at` leaves the light pointing where it last did.
        /// Refused with E_NOT_LIGHT for an entity that is not a light.
        0x00E1 LightAim light_aim { entity: Entity, at: Entity, target: Vec3 }
    }
    events {
        /// The first event of the first tick.
        0x0001 Started started { seed: u64 }
        /// The move axis changed; x right, y up (north), each in [−1, 1].
        0x0002 Axis axis { action: u16, x: f32, y: f32 }
        0x0003 Button button { action: u16, down: bool }
        /// A press of `action` still waiting to be consumed, `age` steps old.
        0x0004 Buffered buffered { action: u16, age: u16 }
        0x0005 Control control { by: Controller }

        0x0010 Arrived arrived { entity: Entity }
        0x0011 Blocked blocked { entity: Entity }
        0x0012 HitWall hit_wall { entity: Entity, normal: Vec3 }
        0x0013 Landed landed { entity: Entity }
        0x0014 TriggerEnter trigger_enter { trigger: u32, entity: Entity }
        0x0015 TriggerExit trigger_exit { trigger: u32, entity: Entity }
        /// A hinged piece the game turns by tweening its scene parameter
        /// `hinge` (`hinge.<node>`) stopped short of `entity`, which stands
        /// in its swing; the tween waits there and goes on once the way is
        /// clear. Once each time it stops for that entity.
        0x0016 HingeBlocked hinge_blocked { hinge: u32, entity: Entity }

        0x0020 ClipFinished clip_finished { entity: Entity, clip: u32 }
        0x0021 ClipMarker clip_marker { entity: Entity, clip: u32, marker: u32 }

        0x0030 TimerFired timer_fired { timer: u32 }
        0x0031 TweenDone tween_done { tween: u32 }
        0x0032 LineDone line_done { line: u32 }
        /// The engine removed an entity, so its slot is free again.
        0x0033 Despawned despawned { entity: Entity }

        0x0040 CutsceneStarted cutscene_started { cutscene: u32 }
        0x0041 CutsceneEnded cutscene_ended { cutscene: u32, skipped: bool }
        /// A cutscene's `call` block, for the game to act on.
        0x0042 CutsceneCall cutscene_call { cutscene: u32, call: u32 }
        /// A cutscene changed a flag.
        0x0043 FlagChanged flag_changed { flag: u32, on: bool }
        /// The active room changed, by the game's `room_switch` or a cutscene's.
        0x0044 RoomSwitched room_switched { room: u32 }

        0x0050 SaveLoaded save_loaded { key: Str, value: Bytes }
        0x0051 SaveLoadDone save_load_done {}
        0x0052 HostMessage host_message { topic: Str, payload: Bytes }
        0x0053 OrientationChanged orientation_changed { orientation: Orientation }
        /// The host asks the game to wind down (back button, app leaving).
        0x0054 ExitRequested exit_requested {}

        /// The engine refused a command, or stopped a move that carried a
        /// visible actor too far in a step (`opcode` 0); `code` says why.
        0x0060 Rejected rejected { opcode: u16, code: Str, entity: Entity }
    }
}
