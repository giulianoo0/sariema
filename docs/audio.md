# Audio

`crates/sariema-audio` is the engine's mixer: buses, streamed music with
loop points, crossfades, stingers, ducking under voice, and loudness
checks at packaging time. A game's sounds are packaged into a sound bank
(`.srms`, `crates/sariema-format/src/sound.rs`) and can be rendered offline
from a timed cue list, so an agent can hear and measure a mix without a
device.

## The model

Two halves joined by a lock-free queue:

- **`Audio`**, on the simulation thread, decodes, opens streams and sends
  commands. Every command carries the **mixer frame** it takes effect on.
  Everything that allocates or frees happens on this side.
- **`Mixer`**, on the audio thread, fills interleaved stereo buffers. It
  never allocates, locks or reads a file (a test arms an allocator guard
  around it), so its output is a pure function of the commands and their
  frames.

The mixer works in **blocks of `block_frames` aligned on its own clock**,
whatever buffer size the device asks for, and splits each block at the
exact frame of every command. A device asking for one frame gets the whole
block holding it mixed, so the mixer runs up to a block ahead of what the
device has read. A command that is in the queue before the block holding
its frame is mixed lands on that frame; then the same commands give the
same samples, bit for bit, with device buffers of 1, 64, 480 or 4096
frames. The offline renderer sends each cue before its block is mixed.

```
simulation ──commands (frame-stamped)──▶ ring ──▶ Mixer ──▶ device
     ▲                                             │
     └──── streams handed back to be freed ◀───────┘
decoder (thread or caller) ──decoded frames──▶ one ring per stream
```

**Voices.** A play becomes a voice on a bus with a gain and a pan
(equal-power for mono sources, balance for stereo ones). At most `voices`
play at once; a play beyond it steals by `steal` among voices of a priority
at most its own, and the victim fades out in one of the `fading_voices`
extra slots. Every start, stop, steal, natural end, pause and stream gap is
faded over at least `declick_s`, so nothing clicks.

**Sounds.** Short sounds are decoded at load (`load: Bank`); voice lines
can be `on_demand`: decoded on `Audio::preload`, released on
`Audio::unload` once no voice plays them. Music and long ambience are
`stream`: only the compressed bytes stay in memory and a decoder fills a
ring of `buffer_s` seconds ahead of the mixer. Decoded audio is converted to
the mixer rate on the decoding side with a Kaiser-windowed sinc; the
mixer only reads audio at its own rate.

**Loops.** Loop points are frames of the decoded sound at its own rate,
encoder delay and padding excluded, so they mean what an editor shows. A
streamed loop seeks back `seek_preroll_frames` before the loop start and
drops frames until the start, so MP3 and Vorbis are settled at the seam;
positions step by the exact ratio of the two rates in integers, so a loop
never drifts. A decoded loop is resampled as a circle.

**Music.** `Audio::music` crossfades from the current track: the new one
comes in over the command's fade, or the sound's `enter_s`, or
`MusicSettings::fade_in_s`; the old one leaves over the command's, or its
own `leave_s`, or `fade_out_s`. Asking for the track already playing does
nothing. `Audio::stinger` drops the track over `stinger_out_s`, holds it
frozen, plays the stinger once and brings the track back over
`stinger_return_s` from the frame where it stopped. Music and stingers play
on the bus their sound declares.

**Ducking.** A rule lowers its `targets` by `depth_db` while any voice plays
on its `key` bus: attack over `attack_s`, then after the key goes quiet a
`hold_s` wait and a release over `release_s`, in dB. The key is the bus's
activity, not its signal, so pauses inside a line do not let the music
swell, and lines closer than the hold do not pump. An interrupted attack or
release continues from the current level with the same slope.

**Pause.** `Audio::pause` fades the master out and freezes every voice
where it is; `resume` fades back in and every voice continues from the
same frame. The mixer clock keeps running.

**Streams that starve.** When a stream's ring runs dry, the voice fades
out over the frames it still has, the mixer counts an underrun and outputs
silence; once `resume_fill` of the ring is filled again it fades back in.
It never outputs garbage and never waits.

## Packaging and loudness

The project file names a sound description with its `audio` key
(`schemas/audio.schema.json`). `sariema audio build` decodes each sound and
measures:

- **integrated loudness** after ITU-R BS.1770-4 (K-weighting, 400 ms blocks
  with 75 % overlap, −70 LUFS absolute gate, −10 LU relative gate); a sound
  shorter than one block is measured as one block. Verified against the
  EBU Tech 3341 reference signals, cases 1 to 5, within 0.1 LU;
- **sample peak**;
- for loops, the **seam step**: the level difference between the
  `seam_window_s` before the loop end and after the loop start.

Each bus may have a `target_lufs`: every sound of the bus is brought to it
before its own `gain_db`. The default buses are music −24, voice −21,
effects −24 and ambience −30 LUFS, which keeps voice clear over music and
ambience under it.

The checks warn (`sariema audio build` and `sariema audio check`; `--strict`
turns any warning into `E_AUDIO_CHECK`):

| Warning | When |
|---|---|
| `W_AUDIO_OVER_MUSIC` | in a mix, a sound of a bus listed in `over_music_db` plays more than that many dB over the loudest music of the mix |
| `W_AUDIO_LOOP_SEAM` | a loop's seam step is larger than `seam_step_db` |

The effective level compared is measured loudness + normalisation + the
sound's gain + its bus gain.

## Settings

`MixerSettings` (every field has a default):

| Field | Default | Meaning |
|---|---|---|
| `sample_rate` | 48 000 | output frames per second |
| `block_frames` | 64 | frames mixed at a time; ducking follows the key bus once per block |
| `buses` | music, voice, effects, ambience at 0 dB | the list of buses |
| `voices` | 32 | sounds audible at once |
| `fading_voices` | 16 | extra slots for voices fading out |
| `steal` | `Oldest` | or `Quietest`, `Refuse` |
| `declick_s` | 0.003 | shortest fade on every start, stop, steal, end, pause, gap |
| `ducking` | voice ducks music and ambience | list of `DuckRule` |
| `commands` | 1024 | commands that may wait in the queue or for their frame |

`DuckRule`: `key` "voice", `targets` music and ambience, `depth_db` −9,
`attack_s` 0.15, `hold_s` 0.25, `release_s` 0.6.

`MusicSettings`: `fade_in_s` 1.2, `fade_out_s` 1.2, `curve` `EqualPower`
(also `Linear`, `SCurve`), `stinger_out_s` 0.13, `stinger_return_s` 1.0.

`StreamSettings`: `buffer_s` 1.0, `resume_fill` 0.5, `seek_preroll_frames`
4096, `poll_ms` 5 (decoding thread), `resampler_half_taps` 16.

`CheckSettings` (in the sound description's `checks`): `over_music_db`
ambience 0 and effects 9, `seam_step_db` 6, `seam_window_s` 0.2,
`music_bus` "music".

## Using it from the CLI

```sh
sariema audio build <project> -o game.srms [--strict] [--json]
sariema audio check <project> [--strict] [--json]
sariema audio inspect game.srms [--json]
sariema audio render game.srms --cues cues.json -o mix.wav \
    [--timeline mix.timeline.json] [--buffer 480] [--rate 48000] [--window 0.1] [--json]
```

`render` plays the cue list (`schemas/cues.schema.json`) through the real
mixer with streams fed in step, writes a 24-bit WAV and a timeline: per
window, each bus's RMS after its gain and ducking, the master's RMS and
peak, and the ducking applied; over the whole render, each bus's and the
master's integrated loudness; the cues applied and any steal, refusal,
underrun or late command.

New error codes: `E_AUDIO_DECODE`, `E_AUDIO_UNKNOWN_SOUND`,
`E_AUDIO_NOT_LOADED`, `E_AUDIO_QUEUE_FULL`, `E_AUDIO_CHECK`,
`E_AUDIO_NOT_FINITE` for a gain or pan that is NaN or infinite (the schemas
cap `gain_db` at +24, so a description cannot ask for one), and
`E_AUDIO_DEVICE` when there is no output device or its stream fails (the
game can carry on silent). A bad bank gives
the pack codes (`E_PACK_BAD_MAGIC`, `E_PACK_VERSION`, `E_PACK_TRUNCATED`,
`E_SCHEMA`).

## Using it from code

```rust
let (settings, sounds) = sariema_audio::runtime(bank, MixerSettings::default());
// Feeding::Manual on the web, with audio.pump() between steps.
let (mut audio, mixer) = Audio::new(settings, sounds, Feeding::Thread)?;
// Audio thread: mixer.render(&mut buffer), or Output::start(mixer, rate)
// with the `device` feature.
let theme = audio.sound("theme")?;
audio.music(step_frame, theme, MusicFades::default(), None)?;
audio.collect(); // once per step: frees what the mixer let go of
```

Stamp a live command at least one block plus one device buffer ahead of
the mixer's clock (`Audio::stats().now`): by the time the command is read,
the mixer may have mixed one more device buffer, rounded up to whole
blocks. A frame already mixed takes effect at the next block and counts as
late, except frame 0, which means "as soon as possible".

## Platforms

Decoding is pure Rust (`symphonia`: MP3, Ogg Vorbis, WAV; MPL-2.0, used
unmodified as a library). The device output is behind the `device` feature
(`cpal`, Apache-2.0); both `sariema-audio` and `--features device` pass
`cargo check` for `aarch64-apple-ios` and `aarch64-linux-android`. The host
app provides what cpal cannot: the `AVAudioSession` category and
interruptions on iOS, `ndk-context` initialised with the JVM and activity on
Android. Pausing on backgrounding is `Audio::pause`.

The web (`wasm32-unknown-unknown`, for a WebView build): `sariema-audio`
passes `cargo check` and clippy there. The web has no threads, so
`Feeding::Thread` does not exist on `wasm32` (using it is a compile error,
not a panic at run time); a web build uses `Feeding::Manual` and calls
`Audio::pump` between steps to decode streams ahead of the mixer. The `device`
feature was not checked for the web; the page provides the output.

Decoding allocates: symphonia gives each packet it reads its own buffer, and
opening a stream builds a decoder. So does decoding a sound loaded on demand.
The engine keeps all of that out of its step (`docs/engine.md`): the step
only plays sounds already decoded, and an audio feed between steps, or a
decoding thread, does the rest. A host that wants no allocation anywhere
decodes its streams whole at load (`audio.predecode_streams` in the engine):
f32 stereo at the mixer's rate, 23 MB a minute at 48 kHz, against 0.58 MB
for a stream.

## Numbers

`cargo run --release -p sariema-audio --example mix_bench -- track.mp3`,
Apple M5, one core, 480-frame callbacks, 48 kHz stereo:

| Voices | ms to mix 1 s, gains steady | ms to mix 1 s, every pan moving |
|---|---|---|
| 4 | 0.20 | 0.41 |
| 32 | 1.20 | 2.44 |
| 128 | 5.21 | 10.69 |

A 3-minute 128 kb/s MP3 streamed: 2.88 MB of compressed bytes plus 0.58 MB
held by its stream (the ring of 1 s and the decoder's buffers); decoding and
mixing 10 s of it takes 68 ms.

## Known limits

- No pitch or playback-rate change, no effects (filters, reverb), no 3D
  attenuation: a voice has a gain and a pan the game sets.
- No limiter on the master: a mix louder than 0 dBFS clips at the device.
- Music transitions start on the frame they are asked for; there is no
  quantisation to beats or bars.
- Channels past the second of a source are dropped.
- ffmpeg's built-in Vorbis encoder declares its end padding as audio; a
  whole-file loop of such a file includes that silence. Give explicit loop
  points, or encode with libvorbis.
- The seam check measures level, not timbre or rhythm: a seam off the beat
  passes.

## What we took from other engines

- **kira** (Bevy ecosystem, `kira` 0.12): the split between a control side
  and a renderer fed through `rtrb` ring buffers, resources handed back to
  the control side to be freed instead of dropped on the audio thread, and a
  decoding scheduler that fills a ring ahead of a streaming sound.
- **Godot** (`AudioStreamInteractive`, audio buses): music transitions with
  separate fade-in and fade-out, `AUTO_ADVANCE_RETURN_TO_HOLD` for a clip
  that returns to the held one (our stinger), buses addressed by name with
  volumes in dB.
- **libebur128**: the K-weighting coefficients for any sample rate
  (`ebur128_init_filter`) and the gating constants.
- **EBU Tech 3341**: the reference signals the loudness meter is tested
  against.
