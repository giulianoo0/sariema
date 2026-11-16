// SPDX-License-Identifier: Apache-2.0

use std::ops::Range;
use std::sync::Mutex;

use ctt_astcenc::bindings::{astcenc_image, astcenc_type_ASTCENC_TYPE_U8};
use ctt_astcenc::{Context, Flags, Preset, Profile, Swizzle, config_init};
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

/// Colour ramps crossed by a fine stripe, inverted inside a hard-edged disc,
/// over an alpha ramp with a transparent rectangle: no 4×4 block is flat.
fn ramps(size: u32) -> RgbaImage {
    let ramp = |v: u32| (v * 255 / size) as u8;
    let half = i64::from(size / 2);
    RgbaImage::from_fn(size, size, |x, y| {
        let rgb = [ramp(x), ramp(y), ((x + y) % 8 * 32) as u8];
        let (dx, dy) = (i64::from(x) - half, i64::from(y) - half);
        let disc = 16 * (dx * dx + dy * dy) < half * half * 4;
        let [r, g, b] = if disc { rgb.map(|c| 255 - c) } else { rgb };
        let hole =
            (size / 8 + 1..size / 3).contains(&x) && (size / 2 + 3..size * 3 / 4).contains(&y);
        let alpha = if hole { 0 } else { 255 - ramp((x + y) / 2) };
        Rgba([r, g, b, alpha])
    })
}

fn sizes(t: &Texture) -> Vec<usize> {
    t.mips.iter().map(Vec::len).collect()
}

fn encode(name: &str, img: &RgbaImage, mode: TextureMode) -> Texture {
    encode_texture(name, img, mode).unwrap()
}

/// Mip 0 of an ASTC texture, decoded to RGBA8.
fn decode_mip0(t: &Texture) -> Vec<u8> {
    let config = config_init(
        Profile::LdrSrgb,
        4,
        4,
        1,
        Preset::Medium,
        Flags::DECOMPRESS_ONLY,
    );
    let mut context = Context::new(&config.unwrap()).unwrap();
    let mut texels = vec![0u8; t.width as usize * t.height as usize * 4];
    let mut data = texels.as_mut_ptr().cast();
    let mut image = astcenc_image {
        dim_x: t.width,
        dim_y: t.height,
        dim_z: 1,
        data_type: astcenc_type_ASTCENC_TYPE_U8,
        data: &mut data,
    };
    context
        .decompress(&t.mips[0], &mut image, Swizzle::IDENTITY)
        .unwrap();
    texels
}

/// Peak signal-to-noise ratio in dB over `channels` of two RGBA8 images.
fn psnr(a: &[u8], b: &[u8], channels: Range<usize>) -> f64 {
    let (mut squares, mut count) = (0.0, 0);
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        for c in channels.clone() {
            squares += (f64::from(p[c]) - f64::from(q[c])).powi(2);
            count += 1;
        }
    }
    10.0 * (255.0_f64.powi(2) * f64::from(count) / squares).log10()
}

/// Collects the warnings logged by every test of this binary.
struct Warnings(Mutex<Vec<String>>);

impl log::Log for Warnings {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            self.0.lock().unwrap().push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

static WARNINGS: Warnings = Warnings(Mutex::new(Vec::new()));

#[test]
fn astc_mip0_has_expected_size() {
    let t = encode("ramps", &ramps(256), TextureMode::Astc4x4);
    assert_eq!(t.format, TextureFormat::Astc4x4Srgb);
    assert_eq!((t.width, t.height), (256, 256));
    assert_eq!(t.mips[0].len(), 64 * 64 * 16);
}

#[test]
fn astc_is_nearly_four_times_smaller() {
    let img = ramps(256);
    let total = |mode| -> usize { sizes(&encode("ramps", &img, mode)).iter().sum() };
    let (astc, rgba8) = (total(TextureMode::Astc4x4), total(TextureMode::Rgba8));
    // 8 bits per texel against 32; padding the 2×2 and 1×1 mips to a block keeps it under 4×.
    assert!(astc as f64 * 3.99 <= rgba8 as f64, "{astc} vs {rgba8}");
}

#[test]
fn tiny_mips_are_one_block() {
    let t = encode("ramps", &ramps(8), TextureMode::Astc4x4);
    assert_eq!(sizes(&t), [4 * 16, 16, 16, 16]);
}

#[test]
fn astc_needs_whole_blocks() {
    for img in [ramps(6), RgbaImage::new(0, 0)] {
        let t = encode("ramps", &img, TextureMode::Astc4x4);
        assert_eq!(t.format, TextureFormat::Rgba8Srgb);
        assert_eq!(t.mips, mip_chain_rgba8(&img));
    }
}

#[test]
fn partial_blocks_fallback_is_logged() {
    log::set_logger(&WARNINGS).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    encode("odd_sides", &ramps(6), TextureMode::Astc4x4);
    let warnings = WARNINGS.0.lock().unwrap();
    let ours: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("odd_sides"))
        .collect();
    assert_eq!(ours.len(), 1, "{warnings:?}");
    assert!(ours[0].contains("6×6"), "{}", ours[0]);
}

#[test]
fn astc_keeps_colour_and_alpha() {
    let img = ramps(256);
    let decoded = decode_mip0(&encode("ramps", &img, TextureMode::Astc4x4));
    let rgb = psnr(img.as_raw(), &decoded, 0..3);
    let alpha = psnr(img.as_raw(), &decoded, 3..4);
    // Preset::Medium measured RGB 42.33 dB and alpha 47.70 dB on this image.
    assert!(
        rgb >= 40.0 && alpha >= 45.0,
        "RGB {rgb:.2} dB, alpha {alpha:.2} dB"
    );
}

#[test]
fn astc_encoding_is_reproducible() {
    let img = ramps(256);
    let encode_astc = || encode("ramps", &img, TextureMode::Astc4x4);
    assert!(encode_astc() == encode_astc());
}
