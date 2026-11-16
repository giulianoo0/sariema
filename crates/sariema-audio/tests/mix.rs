// SPDX-License-Identifier: Apache-2.0

mod common;

use std::f32::consts::FRAC_PI_4;

use common::*;
use sariema_audio::{
    AudioError, DuckRule, MixerSettings, MusicFades, PlayParams, Steal, db_to_gain,
};

/// Centre gain of a mono source under the equal-power pan law.
fn centre() -> f32 {
    FRAC_PI_4.cos()
}

fn counter(frames: usize) -> Vec<f32> {
    (0..frames).map(|n| n as f32 / 1e6).collect()
}

#[test]
fn silence_in_silence_out() {
    let (mut audio, mut mixer) = setup(
        quiet_settings(),
        vec![Def::mono("hush", EFFECTS, vec![0.0; 4800])],
    );
    let idle = render(&mut audio, &mut mixer, 3000, 480);
    assert!(idle.iter().all(|&s| s == 0.0));
    let hush = audio.sound("hush").unwrap();
    audio.play(3000, hush, PlayParams::default()).unwrap();
    let out = render(&mut audio, &mut mixer, 6000, 480);
    assert!(out.iter().all(|&s| s == 0.0));
}

#[test]
fn bus_voice_and_master_gains_multiply() {
    let (mut audio, mut mixer) = setup(
        quiet_settings(),
        vec![Def::mono("dc", EFFECTS, vec![0.5; 9600])],
    );
    let dc = audio.sound("dc").unwrap();
    audio.set_bus(0, EFFECTS, -6.0, 0.0).unwrap();
    audio.set_master(0, -12.0, 0.0).unwrap();
    let params = PlayParams {
        gain: 0.5,
        ..PlayParams::default()
    };
    audio.play(0, dc, params).unwrap();
    let out = render(&mut audio, &mut mixer, 4800, 480);
    let expected = 0.5 * 0.5 * centre() * db_to_gain(-6.0) * db_to_gain(-12.0);
    for &s in &out[2 * 200..] {
        assert!((s - expected).abs() < 1e-6, "{s} vs {expected}");
    }
}

#[test]
fn play_lands_on_its_exact_frame() {
    for buffer in [64, 100, 4096] {
        let (mut audio, mut mixer) = setup(
            quiet_settings(),
            vec![Def::mono("dc", EFFECTS, vec![0.5; 9600])],
        );
        let dc = audio.sound("dc").unwrap();
        audio.play(1000, dc, PlayParams::default()).unwrap();
        let out = left(&render(&mut audio, &mut mixer, 2000, buffer));
        assert!(out[..1001].iter().all(|&s| s == 0.0), "buffer {buffer}");
        assert!(out[1001] > 0.0, "buffer {buffer}");
    }
}

#[test]
fn loop_seam_is_continuous_for_100_loops() {
    // 480-frame period: the loop region holds whole periods, so the looped
    // output must be the same endless sine.
    let period = 480;
    let wave = |n: usize| (std::f32::consts::TAU * (n % period) as f32 / period as f32).sin() * 0.5;
    let sound: Vec<f32> = (0..6720).map(wave).collect();
    let (start, end) = (960u64, 5760u64);
    let def = Def::mono("loop", AMBIENCE, sound).looped(start, end);
    let (mut audio, mut mixer) = setup(quiet_settings(), vec![def]);
    let id = audio.sound("loop").unwrap();
    audio.play(0, id, PlayParams::default()).unwrap();
    let frames = 960 + 100 * 4800 + 100;
    let out = left(&render(&mut audio, &mut mixer, frames, 480));
    let declick = 144;
    for (n, &s) in out.iter().enumerate().skip(declick) {
        assert_eq!(s, wave(n) * centre(), "frame {n}");
    }
}

#[test]
fn equal_power_crossfade_keeps_the_level_and_ends_on_the_frame() {
    let a = Def::mono("a", MUSIC, sine(300.0, 0.5, RATE, 3 * 48_000));
    let b = Def::mono("b", MUSIC, sine(470.0, 0.5, RATE, 3 * 48_000));
    let (mut audio, mut mixer) = setup(quiet_settings(), vec![a, b]);
    let (ia, ib) = (audio.sound("a").unwrap(), audio.sound("b").unwrap());
    let fast = MusicFades {
        fade_in_s: Some(0.0),
        ..MusicFades::default()
    };
    audio.music(0, ia, fast, None).unwrap();
    let fades = MusicFades {
        fade_in_s: Some(0.8),
        fade_out_s: Some(0.8),
        curve: None,
    };
    audio.music(48_000, ib, fades, None).unwrap();
    let out = left(&render(&mut audio, &mut mixer, 2 * 48_000 + 48_000, 480));
    let steady = rms_db(&out[24_000..48_000]);
    for w in out[24_000..140_000].chunks(2400) {
        let level = rms_db(w);
        assert!((level - steady).abs() < 0.5, "{level} vs {steady}");
    }
    let end = 48_000 + 38_400;
    let wave = sine(470.0, 0.5, RATE, 3 * 48_000);
    for n in end..out.len() {
        assert_eq!(out[n], wave[n - 48_000] * centre(), "frame {n}");
    }
    assert_ne!(out[end - 1], wave[end - 1 - 48_000] * centre());
}

fn duck_settings() -> MixerSettings {
    MixerSettings {
        ducking: vec![DuckRule::default()],
        ..MixerSettings::default()
    }
}

/// Music held at a constant level so the output shows the duck gain alone;
/// the voice lines are silent but still count as playing.
fn ducked(lines: &[(u64, usize)], frames: usize) -> Vec<f32> {
    let music = Def::mono("music", MUSIC, vec![0.5; 48_000]);
    let mut defs = vec![music];
    let names = ["l0", "l1", "l2"];
    for (i, &(_, len)) in lines.iter().enumerate() {
        defs.push(Def::mono(names[i], VOICE, vec![0.0; len]));
    }
    let (mut audio, mut mixer) = setup(duck_settings(), defs);
    let m = audio.sound("music").unwrap();
    audio
        .music(
            0,
            m,
            MusicFades {
                fade_in_s: Some(0.0),
                ..MusicFades::default()
            },
            Some(true),
        )
        .unwrap();
    for (i, &(at, _)) in lines.iter().enumerate() {
        let line = audio.sound(names[i]).unwrap();
        audio.play(at, line, PlayParams::default()).unwrap();
    }
    left(&render(&mut audio, &mut mixer, frames, 480))
}

#[test]
fn ducking_depth_attack_hold_and_release_match_the_settings() {
    let out = ducked(&[(48_000, 48_000)], 160_000);
    let level = 0.5 * centre();
    let at = |n: usize, db: f32| {
        let expected = level * db_to_gain(db);
        assert!(
            (out[n] - expected).abs() < 1e-5,
            "frame {n}: {} vs {expected} ({db} dB)",
            out[n]
        );
    };
    at(47_999, 0.0);
    at(48_000 + 3_600, -4.5);
    at(48_000 + 7_200, -9.0);
    // The line ends at 96 000; the duck holds 0.25 s, then releases over 0.6 s.
    at(96_000 + 12_000 - 1, -9.0);
    at(108_000 + 14_400, -4.5);
    at(108_000 + 28_800, 0.0);
    at(150_000, 0.0);
}

#[test]
fn overlapping_and_close_lines_do_not_pump() {
    // Overlap from 1.8 s to 2 s, then a gap of 0.2 s, shorter than the hold.
    let out = ducked(
        &[(48_000, 48_000), (86_400, 48_000), (144_000, 24_000)],
        168_000,
    );
    let depth = 0.5 * centre() * db_to_gain(-9.0);
    for (n, &s) in out.iter().enumerate().take(168_000).skip(48_000 + 7_200) {
        assert!((s - depth).abs() < 1e-5, "frame {n}: {s} vs {depth}");
    }
}

#[test]
fn starts_stops_steals_and_ends_never_click() {
    let settings = MixerSettings {
        voices: 1,
        ..quiet_settings()
    };
    let amp = 0.9;
    let cosine: Vec<f32> = sine(1000.0, amp, RATE, 24_000)
        .iter()
        .enumerate()
        .map(|(n, _)| amp * (std::f32::consts::TAU * 1000.0 * n as f32 / RATE as f32).cos())
        .collect();
    let defs = vec![
        Def::mono("a", EFFECTS, cosine.clone()),
        Def::mono("b", EFFECTS, cosine.clone()),
        Def::mono("short", EFFECTS, cosine[..5_000].to_vec()),
    ];
    let (mut audio, mut mixer) = setup(settings, defs);
    let (a, b, short) = (
        audio.sound("a").unwrap(),
        audio.sound("b").unwrap(),
        audio.sound("short").unwrap(),
    );
    let va = audio.play(1_000, a, PlayParams::default()).unwrap();
    audio.stop(5_000, va, 0.0).unwrap();
    audio.play(8_000, a, PlayParams::default()).unwrap();
    audio.play(12_345, b, PlayParams::default()).unwrap();
    audio.play(30_000, short, PlayParams::default()).unwrap();
    let out = left(&render(&mut audio, &mut mixer, 40_000, 480));
    assert_eq!(audio.stats().steals, 2);
    let natural = amp * centre() * std::f32::consts::TAU * 1000.0 / RATE as f32;
    let jump = max_jump(&out);
    assert!(jump < natural * 1.1, "{jump} vs natural {natural}");
    let unfaded = amp * centre();
    assert!(jump < unfaded / 4.0);
}

#[test]
fn refuse_policy_and_priority_drop_plays() {
    let settings = MixerSettings {
        voices: 1,
        steal: Steal::Refuse,
        ..quiet_settings()
    };
    let defs = vec![Def::mono("a", EFFECTS, vec![0.1; 9600])];
    let (mut audio, mut mixer) = setup(settings, defs);
    let a = audio.sound("a").unwrap();
    audio.play(0, a, PlayParams::default()).unwrap();
    audio.play(10, a, PlayParams::default()).unwrap();
    render(&mut audio, &mut mixer, 480, 480);
    assert_eq!((audio.stats().refused, audio.stats().steals), (1, 0));

    let settings = MixerSettings {
        voices: 1,
        ..quiet_settings()
    };
    let defs = vec![Def::mono("a", EFFECTS, vec![0.1; 9600])];
    let (mut audio, mut mixer) = setup(settings, defs);
    let a = audio.sound("a").unwrap();
    let high = PlayParams {
        priority: 200,
        ..PlayParams::default()
    };
    audio.play(0, a, high).unwrap();
    audio.play(10, a, PlayParams::default()).unwrap();
    render(&mut audio, &mut mixer, 480, 480);
    assert_eq!((audio.stats().refused, audio.stats().steals), (1, 0));
}

#[test]
fn pause_resume_continues_from_the_same_frame() {
    let (mut audio, mut mixer) = setup(
        quiet_settings(),
        vec![Def::mono("count", EFFECTS, counter(200_000))],
    );
    let count = audio.sound("count").unwrap();
    audio.play(0, count, PlayParams::default()).unwrap();
    let (pause, resume, declick) = (20_000u64, 50_000u64, 144usize);
    audio.pause(pause).unwrap();
    audio.resume(resume).unwrap();
    let out = left(&render(&mut audio, &mut mixer, 80_000, 333));
    let frozen = pause as usize + declick;
    assert!(out[frozen..resume as usize].iter().all(|&s| s == 0.0));
    for k in declick..20_000 {
        let n = resume as usize + k;
        let expected = (frozen + k) as f32 / 1e6 * centre();
        assert_eq!(out[n], expected, "frame {n}");
    }
}

#[test]
fn stinger_drops_the_music_and_returns_where_it_was() {
    let defs = vec![
        Def::mono("theme", MUSIC, counter(400_000)),
        Def::mono("jingle", MUSIC, vec![0.0; 24_000]),
    ];
    let (mut audio, mut mixer) = setup(quiet_settings(), defs);
    let (theme, jingle) = (
        audio.sound("theme").unwrap(),
        audio.sound("jingle").unwrap(),
    );
    let instant = MusicFades {
        fade_in_s: Some(0.0),
        ..MusicFades::default()
    };
    audio.music(0, theme, instant, Some(false)).unwrap();
    audio.stinger(48_000, jingle).unwrap();
    let out = left(&render(&mut audio, &mut mixer, 200_000, 480));
    let (drop, back, ret) = (6_240usize, 48_000 + 24_000usize, 48_000usize);
    assert!(out[48_000 + drop..back].iter().all(|&s| s == 0.0));
    for k in ret..ret + 20_000 {
        let expected = (48_000 + drop + k) as f32 / 1e6 * centre();
        assert_eq!(out[back + k], expected, "frame {}", back + k);
    }
}

#[test]
fn on_demand_sounds_need_a_preload_and_outlive_their_unload() {
    let mut line = Def::mono("line", VOICE, vec![0.2; 24_000]);
    line.on_demand = true;
    let (mut audio, mut mixer) = setup(quiet_settings(), vec![line]);
    let id = audio.sound("line").unwrap();
    assert_eq!(
        audio.play(0, id, PlayParams::default()),
        Err(AudioError::NotLoaded("line".into()))
    );
    assert_eq!(audio.loaded_bytes(), 0);
    audio.preload(id).unwrap();
    assert_eq!(audio.loaded_bytes(), 24_000 * 4);
    audio.play(0, id, PlayParams::default()).unwrap();
    render(&mut audio, &mut mixer, 4800, 480);
    audio.unload(id);
    audio.collect();
    assert_eq!(audio.loaded_bytes(), 24_000 * 4, "a voice still plays it");
    let out = left(&render(&mut audio, &mut mixer, 4800, 480));
    assert!(
        out.iter().all(|&s| s > 0.0),
        "unloading does not cut the voice"
    );
    render(&mut audio, &mut mixer, 24_000, 480);
    audio.collect();
    assert_eq!(audio.loaded_bytes(), 0);
    assert!(matches!(
        audio.play(0, id, PlayParams::default()),
        Err(AudioError::NotLoaded(_))
    ));
}

#[test]
fn settings_naming_a_missing_bus_are_rejected() {
    let settings = MixerSettings {
        ducking: vec![DuckRule {
            key: "dialogue".into(),
            ..DuckRule::default()
        }],
        ..MixerSettings::default()
    };
    let sounds = Vec::new();
    let error = sariema_audio::Audio::new(settings, sounds, sariema_audio::Feeding::Manual).err();
    assert!(matches!(error, Some(AudioError::Settings(why)) if why.contains("dialogue")));
}
