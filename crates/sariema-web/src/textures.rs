// SPDX-License-Identifier: Apache-2.0

//! Which build of a room's pack a device loads. A game for the web ships each
//! room built with ASTC 4×4 textures, which phones and Apple GPUs sample, and
//! with RGBA8 textures for the rest; the page fetches one of the two.

use sariema_format::{RoomPack, TextureFormat};

/// A room pack's texture encoding, by the suffix of its file:
/// `room.srma` for ASTC, `room.rgba8.srma` for RGBA8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Astc,
    Rgba8,
}

impl Variant {
    pub fn name(self) -> &'static str {
        match self {
            Variant::Astc => "astc",
            Variant::Rgba8 => "rgba8",
        }
    }

    /// The pack file of `room` in this variant.
    pub fn file(self, room: &str) -> String {
        match self {
            Variant::Astc => format!("{room}.srma"),
            Variant::Rgba8 => format!("{room}.rgba8.srma"),
        }
    }
}

/// The variants a device with `features` samples, the one it prefers first:
/// ASTC takes a quarter of the memory where it can be sampled.
pub fn variants(features: wgpu::Features) -> &'static [Variant] {
    if features.contains(wgpu::Features::TEXTURE_COMPRESSION_ASTC) {
        &[Variant::Astc, Variant::Rgba8]
    } else {
        &[Variant::Rgba8]
    }
}

/// The variant `room` was built in; a room without textures is either.
pub fn variant_of(room: &RoomPack) -> Option<Variant> {
    room.textures
        .iter()
        .map(|t| match t.format {
            TextureFormat::Astc4x4Srgb => Variant::Astc,
            TextureFormat::Rgba8Srgb => Variant::Rgba8,
        })
        .max_by_key(|v| *v == Variant::Astc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_without_astc_takes_rgba8_only() {
        assert_eq!(variants(wgpu::Features::empty()), [Variant::Rgba8]);
        let astc = wgpu::Features::TEXTURE_COMPRESSION_ASTC;
        assert_eq!(variants(astc), [Variant::Astc, Variant::Rgba8]);
        assert_eq!(
            variants(wgpu::Features::TEXTURE_COMPRESSION_BC),
            [Variant::Rgba8]
        );
    }

    #[test]
    fn files_are_named_by_variant() {
        assert_eq!(Variant::Astc.file("origem-s5"), "origem-s5.srma");
        assert_eq!(Variant::Rgba8.file("origem-s5"), "origem-s5.rgba8.srma");
    }

    #[test]
    fn the_demo_room_is_rgba8() {
        let room = sariema_format::demo::demo_room();
        assert_eq!(variant_of(&room), Some(Variant::Rgba8));
    }
}
