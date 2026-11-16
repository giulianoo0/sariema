// SPDX-License-Identifier: Apache-2.0

use sariema_format::{actor, collision, sound};

const MAGICS: [[u8; 4]; 4] = [
    sariema_format::MAGIC,
    collision::MAGIC,
    actor::MAGIC,
    sound::MAGIC,
];

fn header(magic: [u8; 4]) -> Vec<u8> {
    let mut bytes = magic.to_vec();
    bytes.extend(1u32.to_le_bytes());
    bytes.push(0);
    bytes
}

/// The error code each decoder gives `bytes`, `None` past the magic check.
fn bad_magic_codes(bytes: &[u8]) -> [Option<&'static str>; 4] {
    [
        sariema_format::decode(bytes)
            .err()
            .filter(|e| matches!(e, sariema_format::FormatError::BadMagic))
            .map(|e| e.code()),
        collision::decode(bytes)
            .err()
            .filter(|e| matches!(e, collision::CollisionError::BadMagic))
            .map(|e| e.code()),
        actor::decode(bytes)
            .err()
            .filter(|e| matches!(e, actor::ActorPackError::BadMagic))
            .map(|e| e.code()),
        sound::decode(bytes)
            .err()
            .filter(|e| matches!(e, sound::BankError::BadMagic))
            .map(|e| e.code()),
    ]
}

#[test]
fn the_four_pack_magics_differ() {
    for (i, a) in MAGICS.iter().enumerate() {
        for b in &MAGICS[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn each_decoder_rejects_the_other_packs_magics() {
    for (i, magic) in MAGICS.iter().enumerate() {
        let codes = bad_magic_codes(&header(*magic));
        for (j, code) in codes.iter().enumerate() {
            let expected = (i != j).then_some("E_PACK_BAD_MAGIC");
            assert_eq!(
                *code,
                expected,
                "decoder {j} on magic {}",
                String::from_utf8_lossy(magic)
            );
        }
    }
}
