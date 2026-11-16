// SPDX-License-Identifier: Apache-2.0

use image::{Rgba, RgbaImage};
use sariema_format::{Texture, TextureFormat};
use sariema_import::{TextureMode, encode_texture, mip_chain_rgba8};

#[test]
fn mip_chain_halves_down_to_one_texel() {
    let texel = [10, 120, 250, 40];
    let mips = mip_chain_rgba8(&RgbaImage::from_pixel(4, 2, Rgba(texel)));
    let sizes: Vec<_> = mips.iter().map(Vec::len).collect();
    assert_eq!(sizes, [4 * 2 * 4, 2 * 4, 4]);
    for mip in &mips {
        assert!(mip.chunks(4).all(|t| t == texel), "{mip:?}");
    }
}

#[test]
fn mips_average_in_linear_light() {
    let img = RgbaImage::from_fn(2, 1, |x, _| Rgba([255 * x as u8; 4]));
    let mips = mip_chain_rgba8(&img);
    // Half of white in linear light is sRGB 188; averaging the stored bytes gives 128.
    assert_eq!(mips[1], [188, 188, 188, 128]);
}

/// Colour ramps crossed by a fine stripe, so no 4×4 block is flat.
fn ramps(size: u32) -> RgbaImage {
    RgbaImage::from_fn(size, size, |x, y| {
        let ramp = |v: u32| (v * 255 / size) as u8;
        Rgba([ramp(x), ramp(y), ((x + y) % 8 * 32) as u8, 255])
    })
}

fn sizes(t: &Texture) -> Vec<usize> {
    t.mips.iter().map(Vec::len).collect()
}

#[test]
fn astc_mip0_has_expected_size() {
    let t = encode_texture("ramps", &ramps(256), TextureMode::Astc4x4);
    assert_eq!(t.format, TextureFormat::Astc4x4Srgb);
    assert_eq!((t.width, t.height), (256, 256));
    assert_eq!(t.mips[0].len(), 64 * 64 * 16);
}

#[test]
fn astc_is_nearly_four_times_smaller() {
    let img = ramps(256);
    let total = |mode| -> usize { sizes(&encode_texture("ramps", &img, mode)).iter().sum() };
    let (astc, rgba8) = (total(TextureMode::Astc4x4), total(TextureMode::Rgba8));
    // 8 bits per texel against 32; padding the 2×2 and 1×1 mips to a block keeps it under 4×.
    assert!(astc as f64 * 3.99 <= rgba8 as f64, "{astc} vs {rgba8}");
}

#[test]
fn tiny_mips_are_one_block() {
    let t = encode_texture("ramps", &ramps(8), TextureMode::Astc4x4);
    assert_eq!(sizes(&t), [4 * 16, 16, 16, 16]);
}

#[test]
fn astc_needs_whole_blocks() {
    for img in [ramps(6), RgbaImage::new(0, 0)] {
        let t = encode_texture("ramps", &img, TextureMode::Astc4x4);
        assert_eq!(t.format, TextureFormat::Rgba8Srgb);
        assert_eq!(t.mips, mip_chain_rgba8(&img));
    }
}
