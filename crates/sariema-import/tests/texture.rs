// SPDX-License-Identifier: Apache-2.0

use image::{Rgba, RgbaImage};
use sariema_import::mip_chain_rgba8;

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
