// SPDX-License-Identifier: Apache-2.0

//! KTX2 textures with Basis Universal data, as `KHR_texture_basisu` stores
//! them in a glTF: ETC1S in BasisLZ and UASTC LDR 4×4, raw or in
//! Zstandard, decoded to RGBA8 the way Binomial's basis_universal
//! transcoder decodes them (Apache-2.0; this is a port of its decoding,
//! checked texel for texel against it). Only level 0 of a 2D texture is
//! read: the importer makes its own mips.

mod bits;
mod etc1s;
mod uastc;

use image::{Rgba, RgbaImage};

const IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
const HEADER_BYTES: usize = 80;
const MODEL_ETC1S: u8 = 163;
const MODEL_UASTC: u8 = 166;
const SUPERCOMPRESSION_NONE: u32 = 0;
const SUPERCOMPRESSION_BASISLZ: u32 = 1;
const SUPERCOMPRESSION_ZSTD: u32 = 2;
const ETC1S_GLOBAL_BYTES: usize = 20;
const ETC1S_IMAGE_BYTES: usize = 20;
const ETC1S_P_FRAME: u32 = 2;
const UASTC_BLOCK_BYTES: usize = 16;

struct File<'a>(&'a [u8]);

impl<'a> File<'a> {
    fn bytes(&self, at: u64, len: u64, what: &str) -> Result<&'a [u8], String> {
        let end = at.checked_add(len).filter(|&e| e <= self.0.len() as u64);
        match end {
            Some(end) => Ok(&self.0[at as usize..end as usize]),
            None => Err(format!("KTX2 {what} lies past the end of the file")),
        }
    }

    fn u32(&self, at: usize) -> Result<u32, String> {
        let b = self.bytes(at as u64, 4, "header")?;
        Ok(u32::from_le_bytes(b.try_into().unwrap_or_default()))
    }

    fn u64(&self, at: usize) -> Result<u64, String> {
        let b = self.bytes(at as u64, 8, "header")?;
        Ok(u64::from_le_bytes(b.try_into().unwrap_or_default()))
    }
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// What a KTX2 file may hold before it is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ktx2Settings {
    /// Texels along the longer side; a file claiming more is refused before
    /// anything is allocated.
    pub max_side: u32,
}

impl Default for Ktx2Settings {
    /// The largest 2D texture WebGPU's and Metal's limits allow.
    fn default() -> Ktx2Settings {
        Ktx2Settings { max_side: 16_384 }
    }
}

/// Level 0 of the KTX2 file `bytes` as RGBA8, the texels as stored (so sRGB
/// for a colour texture), within [`Ktx2Settings::default`].
pub fn decode_ktx2(bytes: &[u8]) -> Result<RgbaImage, String> {
    decode_ktx2_with(bytes, &Ktx2Settings::default())
}

/// [`decode_ktx2`] within `settings`.
pub fn decode_ktx2_with(bytes: &[u8], settings: &Ktx2Settings) -> Result<RgbaImage, String> {
    let file = File(bytes);
    if bytes.get(..IDENTIFIER.len()) != Some(&IDENTIFIER[..]) {
        return Err("not a KTX2 file".into());
    }
    let vk_format = file.u32(12)?;
    let (width, height, depth) = (file.u32(20)?, file.u32(24)?, file.u32(28)?);
    let (layers, faces) = (file.u32(32)?, file.u32(36)?);
    let supercompression = file.u32(44)?;
    if vk_format != 0 {
        return Err(format!(
            "KTX2 in Vulkan format {vk_format} holds no Basis Universal data"
        ));
    }
    if width == 0 || height == 0 || depth > 0 || layers > 1 || faces != 1 {
        return Err(format!(
            "KTX2 of {width}×{height}×{depth}, {layers} layers and {faces} faces is not a 2D texture"
        ));
    }
    let held = (width as usize)
        .checked_mul(height as usize)
        .and_then(|texels| texels.checked_mul(4));
    if width.max(height) > settings.max_side || held.is_none() {
        return Err(format!(
            "KTX2 of {width}×{height} is larger than {} texels a side",
            settings.max_side
        ));
    }
    let dfd = file.bytes(
        u64::from(file.u32(48)?),
        u64::from(file.u32(52)?),
        "data format",
    )?;
    if dfd.len() < 28 {
        return Err("KTX2 data format descriptor too short".into());
    }
    let model = dfd[12];
    let samples = (usize::from(u16::from_le_bytes([dfd[10], dfd[11]])).saturating_sub(24)) / 16;
    let level = file.bytes(
        file.u64(HEADER_BYTES)?,
        file.u64(HEADER_BYTES + 8)?,
        "level 0",
    )?;
    let blocks = (width.div_ceil(4) as usize, height.div_ceil(4) as usize);
    let mut image = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));
    match (model, supercompression) {
        (MODEL_ETC1S, SUPERCOMPRESSION_BASISLZ) => {
            let levels = file.u32(40)?.max(1) as usize;
            let sgd = file.bytes(file.u64(64)?, file.u64(72)?, "supercompression data")?;
            decode_etc1s(sgd, levels, level, samples == 2, blocks, &mut image)?;
        }
        (MODEL_UASTC, SUPERCOMPRESSION_NONE | SUPERCOMPRESSION_ZSTD) => {
            let unpacked;
            let data = if supercompression == SUPERCOMPRESSION_ZSTD {
                unpacked = zstd::stream::decode_all(level)
                    .map_err(|e| format!("KTX2 level 0 in Zstandard: {e}"))?;
                &unpacked[..]
            } else {
                level
            };
            decode_uastc(data, blocks, &mut image)?;
        }
        (MODEL_ETC1S | MODEL_UASTC, s) => {
            return Err(format!("KTX2 colour model {model} in supercompression {s}"));
        }
        _ => {
            return Err(format!(
                "KTX2 colour model {model} is neither ETC1S ({MODEL_ETC1S}) nor UASTC LDR 4×4 ({MODEL_UASTC})"
            ));
        }
    }
    Ok(image)
}

fn decode_etc1s(
    sgd: &[u8],
    levels: usize,
    level: &[u8],
    alpha: bool,
    blocks: (usize, usize),
    image: &mut RgbaImage,
) -> Result<(), String> {
    let short = || "KTX2 ETC1S global data too short".to_string();
    let header = sgd.get(..ETC1S_GLOBAL_BYTES).ok_or_else(short)?;
    let endpoints = usize::from(u16::from_le_bytes([header[0], header[1]]));
    let selectors = usize::from(u16::from_le_bytes([header[2], header[3]]));
    let lengths = [4, 8, 12].map(|at| le32(header, at) as usize);
    let images_end = ETC1S_GLOBAL_BYTES + ETC1S_IMAGE_BYTES * levels;
    let desc = sgd.get(ETC1S_GLOBAL_BYTES..images_end).ok_or_else(short)?;
    let [flags, rgb_at, rgb_len, alpha_at, alpha_len] =
        [0, 4, 8, 12, 16].map(|at| le32(desc, at) as usize);
    if flags as u32 & ETC1S_P_FRAME != 0 {
        return Err("KTX2 ETC1S video frames are not textures".into());
    }
    let mut at = images_end;
    let mut part = |len: usize| {
        let p = sgd.get(at..at + len).ok_or_else(short);
        at += len;
        p
    };
    let (endpoint_data, selector_data, tables) =
        (part(lengths[0])?, part(lengths[1])?, part(lengths[2])?);
    let codebook = etc1s::Codebook::read(
        (endpoint_data, endpoints),
        (selector_data, selectors),
        tables,
    )?;
    let slice = |at: usize, len: usize| {
        level
            .get(at..at.saturating_add(len))
            .ok_or_else(|| "KTX2 ETC1S slice lies past its level".to_string())
    };
    codebook.decode(
        slice(rgb_at, rgb_len)?,
        blocks,
        |bx, by, colors, selector| {
            put(image, bx, by, |i| {
                let [r, g, b] = colors[select(selector, i)];
                [Some(r), Some(g), Some(b), None]
            });
        },
    )?;
    if alpha {
        codebook.decode(
            slice(alpha_at, alpha_len)?,
            blocks,
            |bx, by, colors, selector| {
                put(image, bx, by, |i| {
                    [None, None, None, Some(colors[select(selector, i)][1])]
                });
            },
        )?;
    }
    Ok(())
}

fn select(selector: &[u8; 4], texel: usize) -> usize {
    usize::from(selector[texel / 4] >> (texel % 4 * 2) & 3)
}

/// Sets the channels `texel` gives of each texel of block `bx`, `by` inside
/// the image.
fn put(image: &mut RgbaImage, bx: usize, by: usize, texel: impl Fn(usize) -> [Option<u8>; 4]) {
    for i in 0..16 {
        let (x, y) = ((bx * 4 + i % 4) as u32, (by * 4 + i / 4) as u32);
        if x < image.width() && y < image.height() {
            let p = image.get_pixel_mut(x, y);
            for (c, v) in texel(i).into_iter().enumerate() {
                if let Some(v) = v {
                    p[c] = v;
                }
            }
        }
    }
}

fn decode_uastc(
    data: &[u8],
    (across, down): (usize, usize),
    image: &mut RgbaImage,
) -> Result<(), String> {
    if data.len() != across * down * UASTC_BLOCK_BYTES {
        return Err(format!(
            "KTX2 UASTC level 0 has {} bytes for {} blocks",
            data.len(),
            across * down
        ));
    }
    for (k, block) in data.as_chunks::<UASTC_BLOCK_BYTES>().0.iter().enumerate() {
        let texels = uastc::decode(block)?;
        put(image, k % across, k / across, |i| texels[i].map(Some));
    }
    Ok(())
}
