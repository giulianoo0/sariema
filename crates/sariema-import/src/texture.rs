// SPDX-License-Identifier: Apache-2.0

//! Mip chains for sRGB colour textures.

/// Every mip level of `img`, largest first, down to 1×1, as RGBA8 sRGB bytes.
/// Each texel averages the 2×2 texels above it in linear light; alpha is
/// averaged as stored. With an odd width or height above 1, the last column
/// or row does not reach the next level.
pub fn mip_chain_rgba8(img: &image::RgbaImage) -> Vec<Vec<u8>> {
    let (mut width, mut height) = img.dimensions();
    let mut level: Vec<[f32; 4]> = img
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            [
                srgb_to_linear(r),
                srgb_to_linear(g),
                srgb_to_linear(b),
                f32::from(a) / 255.0,
            ]
        })
        .collect();
    let mut mips = vec![img.as_raw().clone()];
    while width > 1 || height > 1 {
        let (w, h) = ((width / 2).max(1), (height / 2).max(1));
        let texel = |x: u32, y: u32| level[(y.min(height - 1) * width + x.min(width - 1)) as usize];
        let next: Vec<[f32; 4]> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| {
                let quad =
                    [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| texel(2 * x + dx, 2 * y + dy));
                std::array::from_fn(|c| quad.iter().map(|t| t[c]).sum::<f32>() / 4.0)
            })
            .collect();
        mips.push(
            next.iter()
                .flat_map(|&[r, g, b, a]| {
                    [
                        linear_to_srgb(r),
                        linear_to_srgb(g),
                        linear_to_srgb(b),
                        (a * 255.0).round() as u8,
                    ]
                })
                .collect(),
        );
        (level, width, height) = (next, w, h);
    }
    mips
}

/// The sRGB transfer function (IEC 61966-2-1), from a stored byte to linear light.
pub(crate) fn srgb_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(l: f32) -> u8 {
    let c = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}
