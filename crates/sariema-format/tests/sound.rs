// SPDX-License-Identifier: Apache-2.0

use sariema_format::sound::{
    BankError, Bus, Container, Load, LoopPoints, Loudness, Mix, Sound, SoundBank, decode, encode,
};

fn bank() -> SoundBank {
    let sound = |name: &str, bus: u32, stream: bool| Sound {
        name: name.into(),
        bus,
        gain_db: -3.0,
        container: Container::Wav,
        sample_rate: 48_000,
        channels: 2,
        frames: 96_000,
        loop_points: Some(LoopPoints {
            start: 4_800,
            end: 96_000,
        }),
        stream,
        load: Load::Bank,
        enter_s: Some(0.8),
        leave_s: Some(2.0),
        loudness: Loudness {
            integrated_lufs: -24.0,
            peak_dbfs: -6.0,
            seam_step_db: Some(0.5),
        },
        data: vec![1, 2, 3, 4],
    };
    SoundBank {
        buses: vec![
            Bus {
                name: "music".into(),
                gain_db: 0.0,
                target_lufs: Some(-24.0),
            },
            Bus {
                name: "ambience".into(),
                gain_db: -2.0,
                target_lufs: None,
            },
        ],
        sounds: vec![sound("mus", 0, true), sound("amb", 1, false)],
        mixes: vec![Mix {
            name: "night".into(),
            sounds: vec![0, 1],
        }],
    }
}

#[test]
fn roundtrip() {
    let b = bank();
    assert_eq!(decode(&encode(&b)).unwrap(), b);
}

#[test]
fn room_pack_magic_is_rejected() {
    let mut bytes = encode(&bank());
    bytes[..4].copy_from_slice(b"SRMA");
    assert_eq!(decode(&bytes), Err(BankError::BadMagic));
    assert_eq!(BankError::BadMagic.code(), "E_PACK_BAD_MAGIC");
}

#[test]
fn other_version_is_rejected() {
    let mut bytes = encode(&bank());
    bytes[4..8].copy_from_slice(&7u32.to_le_bytes());
    assert_eq!(decode(&bytes), Err(BankError::Version { found: 7 }));
}

#[test]
fn every_truncation_is_an_error() {
    let bytes = encode(&bank());
    for n in 0..bytes.len() {
        assert!(decode(&bytes[..n]).is_err(), "{n}");
    }
}

#[test]
fn huge_length_prefix_is_an_error() {
    let mut bytes = encode(&bank())[..8].to_vec();
    bytes.extend([0xFF; 16]);
    assert!(decode(&bytes).is_err());
}

fn invalid(change: impl FnOnce(&mut SoundBank)) -> String {
    let mut b = bank();
    change(&mut b);
    match decode(&encode(&b)) {
        Err(BankError::Invalid(why)) => why,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn dangling_bus_is_invalid() {
    assert!(invalid(|b| b.sounds[0].bus = 2).contains("bus 2"));
}

#[test]
fn loop_past_the_end_is_invalid() {
    let why = invalid(|b| b.sounds[0].loop_points.as_mut().unwrap().end = 96_001);
    assert!(why.contains("4800..96001"), "{why}");
}

#[test]
fn empty_loop_is_invalid() {
    invalid(|b| b.sounds[1].loop_points = Some(LoopPoints { start: 9, end: 9 }));
}

#[test]
fn duplicate_names_are_invalid() {
    assert!(invalid(|b| b.sounds[1].name = "mus".into()).contains("mus"));
}

#[test]
fn dangling_mix_sound_is_invalid() {
    invalid(|b| b.mixes[0].sounds.push(5));
}

#[test]
fn non_finite_gain_is_invalid() {
    invalid(|b| b.sounds[0].gain_db = f32::NAN);
}

#[test]
fn silent_sound_measures_minus_infinity() {
    let mut b = bank();
    b.sounds[0].loudness.integrated_lufs = f32::NEG_INFINITY;
    b.sounds[0].loudness.peak_dbfs = f32::NEG_INFINITY;
    assert_eq!(decode(&encode(&b)).unwrap(), b);
}
