// SPDX-License-Identifier: Apache-2.0

//! Room packs of the versions before [`VERSION`](crate::VERSION), read as
//! the current one. Each added a field at the end of the room (7 the
//! character light, 8 the fog, 9 the cutaways) or, 10, changed the
//! cutaways' anchors from a point to a wall.

use serde::{Deserialize, Serialize};

use crate::{
    CharacterLight, CutAnchor, CutLine, CutStyle, Cutaway, Fog, OLDEST_VERSION, RoomPack, VERSION,
};

const _: () = assert!(OLDEST_VERSION == 6 && VERSION == 10);

/// The encoded fields that the body of a pack of version `found` lacks at
/// its end, each as a room without it: no character light, no fog and no
/// cutaway.
pub(crate) fn missing_fields(found: u32) -> Vec<u8> {
    let mut out = Vec::new();
    // Writing to a Vec cannot run out of room, so postcard cannot fail.
    let fail = "postcard cannot fail to serialize a default field";
    if found < 7 {
        out = postcard::to_extend(&CharacterLight::default(), out).expect(fail);
    }
    if found < 8 {
        out = postcard::to_extend(&Fog::default(), out).expect(fail);
    }
    if found < 9 {
        out = postcard::to_extend(&Vec::<Cutaway9>::new(), out).expect(fail);
    }
    out
}

/// Before version 8 the fog, off, took the colour of the background's bottom.
pub(crate) fn upgrade(found: u32, room: &mut RoomPack) {
    if found < 8 {
        room.fog.color = room.background.bottom;
    }
}

/// A [`Cutaway`] of version 9, whose draws on a wall were judged whole at a
/// point of it.
#[derive(Deserialize, Serialize)]
pub(crate) struct Cutaway9 {
    name: String,
    draws: Vec<u32>,
    height: f32,
    line: CutLine,
    radius: f32,
    soft: f32,
    cap: Option<[f32; 3]>,
    cut_shadows: bool,
    anchors: Vec<Option<[f32; 2]>>,
    style: CutStyle,
}

impl From<Cutaway9> for Cutaway {
    fn from(old: Cutaway9) -> Cutaway {
        let point = |at: [f32; 2]| CutAnchor {
            at,
            along: [0.0; 2],
        };
        Cutaway {
            name: old.name,
            draws: old.draws,
            height: old.height,
            line: old.line,
            radius: old.radius,
            soft: old.soft,
            cap: old.cap,
            cut_shadows: old.cut_shadows,
            anchors: old.anchors.into_iter().map(|a| a.map(point)).collect(),
            style: old.style,
        }
    }
}
