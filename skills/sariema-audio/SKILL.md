---
name: sariema-audio
description: Use when a Sariema game needs music, ambience, sound effects or voice lines, when its mix sounds tense, flat, too loud or buried, when loops click or jump at the seam, when voice is hard to hear over music, or when you must verify a mix without an audio device.
---

# Sariema audio

## Overview

Sounds are declared in a JSON sound description, packaged into a sound
bank (`.srms`) that is measured on the way, and played by a mixer with
buses, streamed music, crossfades, stingers and ducking. You cannot hear,
so you **render cue lists offline** and read numbers: integrated loudness
per bus, a level timeline, and warnings the packaging checks raise.

Core rule: **music is the floor of the mix.** Ambience sits under it,
effects and jingles a little over it, voice clearly over it with the music
ducked. The default bus targets already do this; most bad mixes come from
overriding them.

## When to use

- Adding or changing any sound of a game.
- "The game feels tense / tiring": check ambience and looping effects over
  the music, and whether a dangerous moment has its own music.
- A loop clicks, jumps in level or drifts.
- Voice lines are hard to hear, or the music swells between lines.
- Not for: synthesising or editing the audio files themselves.

## Workflow

1. Point `sariema.json` at the description: `"audio": "audio.json"`.
2. Write `audio.json` (`schemas/audio.schema.json`). Leave `buses` out to
   get the defaults (music −24 LUFS, voice −21, effects −24, ambience −30).
3. `sariema audio check <project> --json` — read `warnings` and each sound's
   `effective_lufs`.
4. `sariema audio build <project> -o game.srms --json`.
5. Write a cue list for a moment of play (`schemas/cues.schema.json`) and
   `sariema audio render game.srms --cues cues.json -o mix.wav --json`.
6. Read `bus_lufs` in the reply and `mix.timeline.json`; listen to
   `mix.wav` with a listening tool if you have one.

## Worked example

`examples/audio` in the engine repository:

```json
{
  "root": "../../crates/sariema-audio/tests/fixtures",
  "sounds": {
    "theme": { "file": "tone.mp3", "bus": "music", "stream": true,
               "loop": { "start": 33075, "end": 77175 }, "enter_s": 0.8, "leave_s": 2.0 },
    "wind":  { "file": "tone.ogg", "bus": "ambience", "stream": true,
               "loop": { "start": 33075, "end": 77175 } },
    "chime": { "file": "tone.ogg", "bus": "effects" },
    "hello": { "file": "tone.mp3", "bus": "voice", "on_demand": true }
  },
  "mixes": { "yard": ["theme", "wind", "chime"] }
}
```

```sh
sariema audio check examples/audio --strict          # 4 sounds, no warnings
sariema audio build examples/audio -o game.srms
sariema audio render game.srms --cues examples/audio/cues.json -o mix.wav
# rendered 8.0 s into mix.wav (-20.7 LUFS, peak -17.0 dBFS;
#   music -26.9, voice -21.3, effects -24.3, ambience -31.7)
```

`cues.json` plays the theme and the wind, preloads `hello` and speaks it at
2 s (the music and wind duck), fires `chime` as a stinger at 5 s (the theme
drops and returns where it was), and stops the wind with a 1 s fade.

## In a game

The game pack carries the bank built from `sariema.json`'s `audio`. The
logic plays by name: `play_sound` (with an emitter, heard from the camera:
full gain within 2 m, then the inverse-distance falloff, panned left and
right) and `play_music`; a cutscene's `sound` step and a line's `voice` do
the same, and voice ducks the music. `sariema run --headless --game …`
mixes every step and prints `audio_hash`, the same run after run with one
build; the bench mix to listen to is still a cue list rendered offline. In
a window the sound plays on the default device; without one the report
gives `E_AUDIO_DEVICE` and the game runs silent.

## Quick reference

| Need | Write |
|---|---|
| Music that loops | `"bus": "music", "stream": true, "loop": {"start": s, "end": e}` (frames of the decoded file at its own rate) |
| A chase that enters fast and leaves slowly | `"enter_s": 0.8, "leave_s": 2.0` on the chase track |
| Long ambience | `"bus": "ambience", "stream": true, "loop": true` |
| Voice line | `"bus": "voice", "on_demand": true`, then a `preload` cue before it plays |
| Jingle over the music | a `stinger` cue; the music drops and comes back |
| Sounds that play together | a `mixes` entry, so the checks compare them |
| Quieter or louder than the bus target | the sound's `gain_db` |

Cue actions (`"do"`): `play`, `stop`, `voice`, `music`, `stinger`,
`music_stop`, `bus`, `master`, `pause`, `resume`, `preload`, `unload`.
`t` is seconds on the mixer clock. `play` takes an `id` that `stop` and
`voice` use, and a `priority` (default 128): footsteps never steal a voice
line of priority 200.

## Reading the results

- `bus_lufs` per bus over the render, after gains and ducking. Ambience
  should sit several LU under music; voice a few LU over it.
- Timeline `windows[i].duck_db`: −9 on music and ambience while a voice
  plays, 0 otherwise. Music swelling between close lines means the lines
  are farther apart than `hold_s`.
- `stats.underruns` must be 0 in a render; `steals` and `refused` say the
  voice limit was hit (raise priorities of what matters).

## Common mistakes

| Mistake | What reports it | Fix |
|---|---|---|
| Ambience or a looping heartbeat over the music | `W_AUDIO_OVER_MUSIC` naming the mix and the dB | lower the sound's `gain_db`; keep the default bus targets |
| Loop points from a player's timeline in seconds × 44100 on a 48 kHz file | `E_SCHEMA` "loops over … but has N frames", or a seam warning | loop points are frames of the decoded file at its own rate; `inspect` shows `sample_rate` |
| A loop seam with a level jump | `W_AUDIO_LOOP_SEAM` with the step in dB | move the points to matching levels or crossfade the seam in the file |
| Playing a voice line without `preload` | `E_AUDIO_NOT_LOADED` | add a `preload` cue (or call `Audio::preload`) |
| A cue naming a sound or id that does not exist | `E_AUDIO_UNKNOWN_SOUND` | check names in `sariema audio inspect game.srms` |
| A misspelt cue action or field | `E_SCHEMA` with the JSON path | follow `schemas/cues.schema.json` |
| A file that is not MP3, Ogg Vorbis or WAV, or is damaged | `E_AUDIO_DECODE` | convert it |
| Whole-file loop of an Ogg made by ffmpeg's built-in Vorbis encoder | a silent gap at every seam | explicit loop points, or encode with libvorbis |
| Setting `target_lufs: null` on every bus to "keep the files as they are" | levels follow whatever the files were mastered at | keep targets and adjust `gain_db` per sound |

`--strict` turns any warning into `E_AUDIO_CHECK` (exit 1): use it in a
game's build so a tense mix cannot ship unnoticed.
