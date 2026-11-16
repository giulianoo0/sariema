// SPDX-License-Identifier: Apache-2.0

//! Mip chains for sRGB colour textures, stored as RGBA8 or ASTC 4×4.

use ctt_astcenc::bindings::{astcenc_image, astcenc_type_ASTCENC_TYPE_U8};
use ctt_astcenc::{Context, Flags, Preset, Profile, Swizzle, config_init};
use image::RgbaImage;
use sariema_format::{Texture, TextureFormat};
use serde::Deserialize;

use crate::ImportError;

/// How a pack stores its colour textures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextureMode {
    Rgba8,
    /// 8 bits per texel; the renderer needs `Features::TEXTURE_COMPRESSION_ASTC`.
    #[default]
    #[serde(rename = "astc")]
    Astc4x4,
}

/// `img` with its mip chain. In [`TextureMode::Astc4x4`] each mip is compressed
/// on its own and a mip under 4×4 takes one block. An image whose sides are not
/// positive multiples of 4 stays RGBA8, with a warning, since the GPU only takes
/// whole blocks.
pub fn encode_texture(
    name: &str,
    img: &RgbaImage,
    mode: TextureMode,
) -> Result<Texture, ImportError> {
    let (width, height) = img.dimensions();
    let mips = mip_chain_rgba8(img);
    let whole_blocks = [width, height]
        .iter()
        .all(|&s| s > 0 && s.is_multiple_of(4));
    let (format, mips) = match mode {
        TextureMode::Astc4x4 if whole_blocks => {
            let blocks = astc_4x4(mips, width, height).map_err(|e| ImportError::Encode {
                texture: name.to_owned(),
                why: e.to_string(),
            })?;
            (TextureFormat::Astc4x4Srgb, blocks)
        }
        TextureMode::Astc4x4 => {
            log::warn!(
                "texture {name} is {width}×{height}, not whole 4×4 blocks, so it stays RGBA8 instead of ASTC"
            );
            (TextureFormat::Rgba8Srgb, mips)
        }
        TextureMode::Rgba8 => (TextureFormat::Rgba8Srgb, mips),
    };
    Ok(Texture {
        name: name.to_owned(),
        width,
        height,
        format,
        mips,
    })
}

/// `mips` are the RGBA8 levels of a `width × height` image, as [`mip_chain_rgba8`] makes them.
fn astc_4x4(
    mips: Vec<Vec<u8>>,
    width: u32,
    height: u32,
) -> Result<Vec<Vec<u8>>, ctt_astcenc::Error> {
    let config = config_init(Profile::LdrSrgb, 4, 4, 1, Preset::Medium, Flags::empty())?;
    let mut context = Context::new(&config)?;
    (0u32..)
        .zip(mips)
        .map(|(level, mip)| {
            let (w, h) = ((width >> level).max(1), (height >> level).max(1));
            let mut rgba =
                RgbaImage::from_raw(w, h, mip).expect("mip_chain_rgba8 makes w × h levels");
            compress_level(&mut context, &mut rgba)
        })
        .collect()
}

/// The only call that hands astcenc texel memory: the safe `Context::compress` reads
/// `dim_x × dim_y` RGBA8 texels through the bare pointer in `astcenc_image`, and `level` holds them.
fn compress_level(
    context: &mut Context,
    level: &mut RgbaImage,
) -> Result<Vec<u8>, ctt_astcenc::Error> {
    let (w, h) = level.dimensions();
    let mut texels = level.as_mut_ptr().cast();
    let mut image = astcenc_image {
        dim_x: w,
        dim_y: h,
        dim_z: 1,
        data_type: astcenc_type_ASTCENC_TYPE_U8,
        data: &mut texels,
    };
    let mut blocks = vec![0; w.div_ceil(4) as usize * h.div_ceil(4) as usize * 16];
    context.compress(&mut image, Swizzle::IDENTITY, &mut blocks)?;
    context.compress_reset()?;
    Ok(blocks)
}

/// Every mip level of `img`, largest first, down to 1×1, as RGBA8 sRGB bytes.
/// Each texel averages the 2×2 texels above it in linear light; alpha is
/// averaged as stored. With an odd width or height above 1, the last column
/// or row does not reach the next level.
pub fn mip_chain_rgba8(img: &RgbaImage) -> Vec<Vec<u8>> {
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
