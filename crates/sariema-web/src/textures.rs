// SPDX-License-Identifier: Apache-2.0

//! Which build of a pack a device loads. `sariema web` builds each room and
//! actor with ASTC 4×4 textures, which phones and Apple GPUs sample, and with
//! RGBA8 textures for the rest; the page fetches one of the two.

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
}
