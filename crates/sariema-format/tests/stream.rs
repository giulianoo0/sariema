// SPDX-License-Identifier: Apache-2.0

use sariema_format::demo::demo_room;
use sariema_format::stream::RoomDecoder;
use sariema_format::{FormatError, RoomPack, decode, encode};

/// The demo room with its meshes and textures repeated, so it has many
/// parts to read.
fn big_room() -> RoomPack {
    let mut room = demo_room();
    for _ in 0..20 {
        room.meshes.push(room.meshes[1].clone());
        room.textures.push(room.textures[0].clone());
    }
    room
}

#[test]
fn a_room_read_a_share_at_a_time_is_the_room_decoded_whole() {
    for room in [demo_room(), big_room()] {
        let bytes = encode(&room);
        let mut decoder = RoomDecoder::new(&bytes[..]);
        let mut shares = 0;
        let mut last = 0;
        let decoded = loop {
            shares += 1;
            if let Some(room) = decoder.step(512).unwrap() {
                break room;
            }
            let (read, all) = decoder.progress();
            assert_eq!(all, bytes.len());
            assert!(read > last, "each share reads something");
            last = read;
        };
        assert_eq!(decoded, decode(&bytes).unwrap());
        assert!(
            shares >= room.textures.len(),
            "{shares} shares of {} bytes",
            bytes.len()
        );
        assert_eq!(decoder.step(512).unwrap(), None, "nothing after the room");
    }
}

#[test]
fn a_share_reads_whole_parts_about_the_budget() {
    let bytes = encode(&big_room());
    let texture = encode(&demo_room()).len();
    let mut decoder = RoomDecoder::new(&bytes[..]);
    while decoder.step(1).unwrap().is_none() {
        let (read, _) = decoder.progress();
        assert!(read <= bytes.len());
    }
    let mut decoder = RoomDecoder::new(&bytes[..]);
    decoder.step(1024).unwrap();
    let (read, _) = decoder.progress();
    assert!(read >= 1024 && read < 1024 + texture, "read {read}");
}

#[test]
fn errors_are_those_of_a_whole_decode() {
    let bytes = encode(&big_room());
    let whole = |bytes: &[u8]| RoomDecoder::decode(bytes).unwrap_err();
    assert_eq!(whole(&bytes[..bytes.len() - 3]), FormatError::Truncated);
    assert_eq!(whole(b"SRMX\x06\0\0\0"), FormatError::BadMagic);
    let mut old = bytes.clone();
    old[4] = 5;
    assert_eq!(whole(&old), FormatError::Version { found: 5 });
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(
        matches!(whole(&longer), FormatError::Invalid(why) if why.contains("unexpected bytes"))
    );

    let mut room = big_room();
    room.meshes[3].indices[0] = 9999;
    let bad = encode(&room);
    assert_eq!(whole(&bad), decode(&bad).unwrap_err());
    let mut decoder = RoomDecoder::new(&bad[..]);
    let error = loop {
        match decoder.step(64) {
            Ok(None) => continue,
            Ok(Some(_)) => panic!("a mesh with a vertex past its end decoded"),
            Err(e) => break e,
        }
    };
    assert!(matches!(&error, FormatError::Invalid(why) if why.contains("mesh 3")));
    let (read, all) = decoder.progress();
    assert!(
        read < all / 2,
        "the bad mesh is found before the textures are read"
    );
    assert_eq!(decoder.step(64), Ok(None), "nothing after an error");
}
