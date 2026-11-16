// SPDX-License-Identifier: Apache-2.0

//! KTX2 textures holding Basis Universal data, as `KHR_texture_basisu` puts
//! them in a glTF: ETC1S in BasisLZ, with or without an alpha slice, and
//! UASTC LDR 4×4, raw or in Zstandard. Small encoders here write each with
//! a few of its modes; the importer must decode every texel to what the
//! encoder meant, close to the source, and store it as RGBA8 or ASTC 4×4
//! like any other texture.

mod common;

use ctt_astcenc::bindings::{astcenc_image, astcenc_type_ASTCENC_TYPE_U8};
use ctt_astcenc::{Context, Flags, Preset, Profile, Swizzle, config_init};
use image::{Rgba, RgbaImage};
use sariema_format::TextureFormat;
use sariema_import::{
    Ktx2Settings, TextureMode, decode_ktx2, decode_ktx2_with, encode_texture, load_kit,
};

use common::write_glb;

/// Smooth colour ramps, a soft disc and, with `alpha`, an alpha ramp.
fn source(width: u32, height: u32, alpha: bool) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
        let d = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt();
        let disc = (1.0 - (d * 4.0 - 0.8).clamp(0.0, 1.0)) * 0.5;
        let c = |t: f32| ((t.clamp(0.0, 1.0)) * 255.0).round() as u8;
        Rgba([
            c(0.2 + 0.6 * u + disc * 0.3),
            c(0.3 + 0.4 * v),
            c(0.7 - 0.5 * u + disc),
            if alpha { c(0.15 + 0.85 * v) } else { 255 },
        ])
    })
}

fn mean_and_max_error(a: &RgbaImage, b: &RgbaImage) -> (f64, u8) {
    assert_eq!(a.dimensions(), b.dimensions());
    let (mut sum, mut max, mut n) = (0.0, 0u8, 0.0);
    for (p, q) in a.pixels().zip(b.pixels()) {
        for c in 0..4 {
            let d = p[c].abs_diff(q[c]);
            sum += f64::from(d);
            max = max.max(d);
            n += 1.0;
        }
    }
    (sum / n, max)
}

#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    len: usize,
}

impl Bits {
    /// `n` bits of `value`, least significant first.
    fn put(&mut self, value: u32, n: u32) {
        for i in 0..n {
            if self.len.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let bit = ((value >> i) & 1) as u8;
            *self.bytes.last_mut().unwrap() |= bit << (self.len % 8);
            self.len += 1;
        }
    }

    /// A Huffman code, its most significant bit first.
    fn code(&mut self, (code, len): (u32, u32)) {
        for i in (0..len).rev() {
            self.put((code >> i) & 1, 1);
        }
    }
}

/// Canonical codes for these code sizes, as DEFLATE assigns them.
fn canonical(sizes: &[u8]) -> Vec<(u32, u32)> {
    let mut count = [0u32; 33];
    for &s in sizes {
        count[s as usize] += 1;
    }
    count[0] = 0;
    let (mut next, mut code) = ([0u32; 33], 0);
    for len in 1..33 {
        code = (code + count[len - 1]) << 1;
        next[len] = code;
    }
    sizes
        .iter()
        .map(|&s| {
            if s == 0 {
                return (0, 0);
            }
            let c = next[s as usize];
            next[s as usize] += 1;
            (c, u32::from(s))
        })
        .collect()
}

/// Code sizes of a complete prefix code over `used`, every other symbol
/// of `total` without a code.
fn complete(total: usize, used: &[usize]) -> Vec<u8> {
    let mut sizes = vec![0u8; total];
    if used.len() == 1 {
        sizes[used[0]] = 1;
        return sizes;
    }
    let k = used.len();
    let long = (usize::BITS - (k - 1).leading_zeros()) as u8;
    let short = (1usize << long) - k;
    for (i, &s) in used.iter().enumerate() {
        sizes[s] = if i < short { long - 1 } else { long };
    }
    sizes
}

const CODELENGTH_ORDER: [usize; 21] = [
    17, 18, 19, 20, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15, 16,
];

/// A Huffman table as Basis Universal stores one: its code sizes, each
/// coded with a table of code-length codes.
fn table(w: &mut Bits, sizes: &[u8]) {
    w.put(sizes.len() as u32, 14);
    let mut used: Vec<usize> = sizes.iter().map(|&s| s as usize).collect();
    used.sort_unstable();
    used.dedup();
    let lengths = complete(21, &used);
    let codes = canonical(&lengths);
    w.put(21, 5);
    for &c in &CODELENGTH_ORDER {
        w.put(u32::from(lengths[c]), 3);
    }
    for &s in sizes {
        w.code(codes[s as usize]);
    }
}

/// Every symbol below `n`, coded.
fn all(n: usize) -> Vec<u8> {
    complete(n, &(0..n).collect::<Vec<_>>())
}

const KTX2_ID: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

/// A data format descriptor: one basic block in `model`, sRGB, with a
/// sample per channel id.
fn dfd(model: u8, channels: &[u8], bytes_plane0: u8) -> Vec<u8> {
    let block = 24 + 16 * channels.len();
    let mut d = Vec::new();
    d.extend(((4 + block) as u32).to_le_bytes());
    d.extend(0u32.to_le_bytes());
    d.extend(2u16.to_le_bytes());
    d.extend((block as u16).to_le_bytes());
    d.extend([model, 1, 2, 0, 3, 3, 0, 0]);
    d.extend([bytes_plane0, 0, 0, 0, 0, 0, 0, 0]);
    for (i, &channel) in channels.iter().enumerate() {
        d.extend(((i * 64) as u16).to_le_bytes());
        d.extend([63, channel, 0, 0, 0, 0]);
        d.extend(0u32.to_le_bytes());
        d.extend(u32::MAX.to_le_bytes());
    }
    d
}

struct Ktx2 {
    width: u32,
    height: u32,
    supercompression: u32,
    dfd: Vec<u8>,
    sgd: Vec<u8>,
    level: Vec<u8>,
    uncompressed: u64,
}

impl Ktx2 {
    fn bytes(&self) -> Vec<u8> {
        let dfd_at = 80 + 24;
        let sgd_at = (dfd_at + self.dfd.len()).next_multiple_of(8);
        let level_at = (sgd_at + self.sgd.len()).next_multiple_of(8);
        let mut f = Vec::new();
        f.extend(KTX2_ID);
        for v in [
            0,
            1,
            self.width,
            self.height,
            0,
            0,
            1,
            1,
            self.supercompression,
            dfd_at as u32,
            self.dfd.len() as u32,
            0,
            0,
        ] {
            f.extend(v.to_le_bytes());
        }
        let sgd_offset = if self.sgd.is_empty() { 0 } else { sgd_at };
        f.extend((sgd_offset as u64).to_le_bytes());
        f.extend((self.sgd.len() as u64).to_le_bytes());
        f.extend((level_at as u64).to_le_bytes());
        f.extend((self.level.len() as u64).to_le_bytes());
        f.extend(self.uncompressed.to_le_bytes());
        f.extend(&self.dfd);
        f.resize(sgd_at, 0);
        f.extend(&self.sgd);
        f.resize(level_at, 0);
        f.extend(&self.level);
        f
    }
}

/// The 4×4 block at `bx`, `by`, edge texels repeated past the image.
fn block(img: &RgbaImage, bx: u32, by: u32) -> [[u8; 4]; 16] {
    let mut out = [[0; 4]; 16];
    for (i, t) in out.iter_mut().enumerate() {
        let x = (bx * 4 + i as u32 % 4).min(img.width() - 1);
        let y = (by * 4 + i as u32 / 4).min(img.height() - 1);
        *t = img.get_pixel(x, y).0;
    }
    out
}

fn put_block(img: &mut RgbaImage, bx: u32, by: u32, texels: &[[u8; 4]; 16]) {
    for (i, t) in texels.iter().enumerate() {
        let (x, y) = (bx * 4 + i as u32 % 4, by * 4 + i as u32 / 4);
        if x < img.width() && y < img.height() {
            img.put_pixel(x, y, Rgba(*t));
        }
    }
}

mod etc1s {
    use super::*;

    const INTEN: [[i32; 4]; 8] = [
        [-8, -2, 2, 8],
        [-17, -5, 5, 17],
        [-29, -9, 9, 29],
        [-42, -13, 13, 42],
        [-60, -18, 18, 60],
        [-80, -24, 24, 80],
        [-106, -33, 33, 106],
        [-183, -47, 47, 183],
    ];

    #[derive(Clone, Copy, PartialEq)]
    pub struct Endpoint {
        color5: [u8; 3],
        inten: u8,
    }

    /// A row a byte, two bits a texel, the left one lowest.
    pub type Selector = [u8; 4];

    fn colors(e: Endpoint) -> [[u8; 3]; 4] {
        let base = e.color5.map(|c| i32::from((c << 3) | (c >> 2)));
        INTEN[e.inten as usize].map(|m| base.map(|b| (b + m).clamp(0, 255) as u8))
    }

    /// The endpoint and selectors nearest these colours.
    fn fit(texels: &[[u8; 3]; 16]) -> (Endpoint, Selector) {
        let mut avg = [0u32; 3];
        for t in texels {
            for c in 0..3 {
                avg[c] += u32::from(t[c]);
            }
        }
        let color5 = avg.map(|s| ((s / 16 * 31 + 127) / 255) as u8);
        let mut best = (u32::MAX, Endpoint { color5, inten: 0 }, [0; 4]);
        for inten in 0..8 {
            let e = Endpoint { color5, inten };
            let palette = colors(e);
            let (mut error, mut selector) = (0, [0u8; 4]);
            for (i, t) in texels.iter().enumerate() {
                let (s, d) = (0..4)
                    .map(|s| {
                        let d: u32 = (0..3)
                            .map(|c| (i32::from(palette[s][c]) - i32::from(t[c])).pow(2) as u32)
                            .sum();
                        (s, d)
                    })
                    .min_by_key(|&(_, d)| d)
                    .unwrap();
                error += d;
                selector[i / 4] |= (s as u8) << (i % 4 * 2);
            }
            if error < best.0 {
                best = (error, e, selector);
            }
        }
        (best.1, best.2)
    }

    pub struct Encoded {
        pub file: Vec<u8>,
        /// What a decoder must give back.
        pub meant: RgbaImage,
    }

    /// `img` in ETC1S; with `alpha`, its alpha in a second slice.
    pub fn encode(img: &RgbaImage, alpha: bool) -> Encoded {
        let (bw, bh) = (img.width().div_ceil(4), img.height().div_ceil(4));
        let (mut endpoints, mut selectors) = (Vec::<Endpoint>::new(), Vec::<Selector>::new());
        let mut index = |e: Endpoint, s: Selector| {
            let ei = endpoints.iter().position(|&x| x == e).unwrap_or_else(|| {
                endpoints.push(e);
                endpoints.len() - 1
            });
            let si = selectors.iter().position(|&x| x == s).unwrap_or_else(|| {
                selectors.push(s);
                selectors.len() - 1
            });
            (ei, si)
        };
        let mut meant = RgbaImage::new(img.width(), img.height());
        let (mut rgb, mut alpha_blocks) = (Vec::new(), Vec::new());
        for by in 0..bh {
            for bx in 0..bw {
                let texels = block(img, bx, by);
                let (e, s) = fit(&texels.map(|t| [t[0], t[1], t[2]]));
                rgb.push(index(e, s));
                let (ae, a_s) = fit(&texels.map(|t| [t[3]; 3]));
                if alpha {
                    alpha_blocks.push(index(ae, a_s));
                }
                let (pc, pa) = (colors(e), colors(ae));
                let out = std::array::from_fn(|i| {
                    let pick = |sel: Selector| (sel[i / 4] >> (i % 4 * 2) & 3) as usize;
                    let c = pc[pick(s)];
                    [c[0], c[1], c[2], if alpha { pa[pick(a_s)][1] } else { 255 }]
                });
                put_block(&mut meant, bx, by, &out);
            }
        }
        let (n, m) = (endpoints.len(), selectors.len());

        let mut w = Bits::default();
        for _ in 0..3 {
            table(&mut w, &[5; 32]);
        }
        table(&mut w, &[3; 8]);
        w.put(0, 1);
        let (five, three) = (canonical(&[5; 32]), canonical(&[3; 8]));
        let (mut prev, mut prev_inten) = ([16u8; 3], 0u8);
        for e in &endpoints {
            w.code(three[(e.inten.wrapping_sub(prev_inten) & 7) as usize]);
            prev_inten = e.inten;
            for c in 0..3 {
                w.code(five[(e.color5[c].wrapping_sub(prev[c]) & 31) as usize]);
                prev[c] = e.color5[c];
            }
        }
        let endpoint_data = w.bytes;

        let mut w = Bits::default();
        w.put(0b100, 3);
        for s in &selectors {
            for &row in s {
                w.put(u32::from(row), 8);
            }
        }
        let selector_data = w.bytes;

        let pred = complete(256, &[255]);
        let mut w = Bits::default();
        table(&mut w, &pred);
        table(&mut w, &all(n));
        table(&mut w, &all(m));
        table(&mut w, &complete(1, &[0]));
        w.put(8, 13);
        let table_data = w.bytes;

        let slice = |blocks: &[(usize, usize)]| {
            let (pred, delta, sel) = (canonical(&pred), canonical(&all(n)), canonical(&all(m)));
            let mut w = Bits::default();
            let mut prev = 0;
            for by in 0..bh as usize {
                for bx in 0..bw as usize {
                    if bx % 2 == 0 && by % 2 == 0 {
                        w.code(pred[255]);
                    }
                    let (e, s) = blocks[by * bw as usize + bx];
                    w.code(delta[(e + n - prev) % n]);
                    prev = e;
                    w.code(sel[s]);
                }
            }
            w.bytes
        };
        let rgb_slice = slice(&rgb);
        let alpha_slice = if alpha {
            slice(&alpha_blocks)
        } else {
            Vec::new()
        };

        let mut sgd = Vec::new();
        sgd.extend((n as u16).to_le_bytes());
        sgd.extend((m as u16).to_le_bytes());
        for len in [
            endpoint_data.len(),
            selector_data.len(),
            table_data.len(),
            0,
        ] {
            sgd.extend((len as u32).to_le_bytes());
        }
        for v in [0, 0, rgb_slice.len(), rgb_slice.len(), alpha_slice.len()] {
            sgd.extend((v as u32).to_le_bytes());
        }
        sgd.extend(&endpoint_data);
        sgd.extend(&selector_data);
        sgd.extend(&table_data);
        let channels: &[u8] = if alpha { &[0, 15] } else { &[0] };
        let file = Ktx2 {
            width: img.width(),
            height: img.height(),
            supercompression: 1,
            dfd: dfd(163, channels, 0),
            sgd,
            level: [rgb_slice, alpha_slice].concat(),
            uncompressed: 0,
        }
        .bytes();
        Encoded { file, meant }
    }
}

mod uastc {
    use super::*;

    const W2: [u32; 4] = [0, 21, 43, 64];
    const W3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];

    fn interpolate(l: u8, h: u8, w: u32) -> u8 {
        let (l, h) = (u32::from(l) * 257, u32::from(h) * 257);
        ((l * (64 - w) + h * w + 32) >> 6 >> 8) as u8
    }

    /// One block: solid in mode 8, opaque in mode 5 (RGB, 3-bit weights),
    /// translucent in mode 14 (RGBA, 2-bit weights); with what it decodes to.
    fn encode_block(texels: &[[u8; 4]; 16]) -> ([u8; 16], [[u8; 4]; 16]) {
        let mut w = Bits::default();
        if texels.iter().all(|t| t == &texels[0]) {
            w.put(0x17, 5);
            for c in texels[0] {
                w.put(u32::from(c), 8);
            }
            let mut bytes = [0; 16];
            bytes[..w.bytes.len()].copy_from_slice(&w.bytes);
            return (bytes, *texels);
        }
        let translucent = texels.iter().any(|t| t[3] < 255);
        let (code, hints, comps, weights): ((u32, u32), u32, usize, &[u32]) = if translucent {
            ((0xD, 5), 23, 4, &W2)
        } else {
            ((0xB, 5), 15, 3, &W3)
        };
        let levels = weights.len();
        let mut lo = [255u8; 4];
        let mut hi = [0u8; 4];
        for t in texels {
            for c in 0..comps {
                lo[c] = lo[c].min(t[c]);
                hi[c] = hi[c].max(t[c]);
            }
        }
        let decoded = |lo: [u8; 4], hi: [u8; 4], l: usize| -> [u8; 4] {
            std::array::from_fn(|c| {
                if c < comps {
                    interpolate(lo[c], hi[c], weights[l])
                } else {
                    255
                }
            })
        };
        let mut chosen: [usize; 16] = std::array::from_fn(|i| {
            (0..levels)
                .min_by_key(|&l| {
                    let d = decoded(lo, hi, l);
                    (0..comps)
                        .map(|c| (i32::from(d[c]) - i32::from(texels[i][c])).pow(2))
                        .sum::<i32>()
                })
                .unwrap()
        });
        if chosen[0] >= levels / 2 {
            std::mem::swap(&mut lo, &mut hi);
            chosen = chosen.map(|l| levels - 1 - l);
        }
        w.put(code.0, code.1);
        w.put(0, hints);
        for c in 0..comps {
            w.put(u32::from(lo[c]), 8);
            w.put(u32::from(hi[c]), 8);
        }
        let bits = levels.trailing_zeros();
        for (i, &l) in chosen.iter().enumerate() {
            w.put(l as u32, if i == 0 { bits - 1 } else { bits });
        }
        let mut bytes = [0; 16];
        bytes[..w.bytes.len()].copy_from_slice(&w.bytes);
        (bytes, chosen.map(|l| decoded(lo, hi, l)))
    }

    pub struct Encoded {
        pub file: Vec<u8>,
        pub meant: RgbaImage,
    }

    /// `img` in UASTC, its blocks in Zstandard when `zstd`.
    pub fn encode(img: &RgbaImage, zstd: bool) -> Encoded {
        let (bw, bh) = (img.width().div_ceil(4), img.height().div_ceil(4));
        let mut meant = RgbaImage::new(img.width(), img.height());
        let mut blocks = Vec::new();
        for by in 0..bh {
            for bx in 0..bw {
                let (bytes, out) = encode_block(&block(img, bx, by));
                blocks.extend(bytes);
                put_block(&mut meant, bx, by, &out);
            }
        }
        let uncompressed = blocks.len() as u64;
        let level = if zstd {
            zstd::bulk::compress(&blocks, 3).unwrap()
        } else {
            blocks
        };
        let translucent = img.pixels().any(|p| p[3] < 255);
        let file = Ktx2 {
            width: img.width(),
            height: img.height(),
            supercompression: if zstd { 2 } else { 0 },
            dfd: dfd(166, &[if translucent { 3 } else { 0 }], 16),
            sgd: Vec::new(),
            level,
            uncompressed,
        }
        .bytes();
        Encoded { file, meant }
    }
}

#[test]
fn an_etc1s_texture_decodes_to_what_its_encoder_meant_and_near_the_source() {
    let src = source(64, 48, false);
    let e = etc1s::encode(&src, false);
    let decoded = decode_ktx2(&e.file).unwrap();
    assert_eq!(decoded, e.meant);
    let (mean, max) = mean_and_max_error(&decoded, &src);
    assert!(mean < 3.0 && max < 40, "mean {mean}, max {max}");
}

#[test]
fn an_etc1s_alpha_slice_is_the_alpha_and_partial_blocks_are_cut() {
    let src = source(18, 10, true);
    let e = etc1s::encode(&src, true);
    let decoded = decode_ktx2(&e.file).unwrap();
    assert_eq!(decoded.dimensions(), (18, 10));
    assert_eq!(decoded, e.meant);
    // A steep alpha ramp on few blocks: ETC1S gives each block one colour
    // and four steps around it.
    let (mean, max) = mean_and_max_error(&decoded, &src);
    assert!(mean < 10.0 && max < 64, "mean {mean}, max {max}");
}

#[test]
fn a_uastc_texture_decodes_to_what_its_encoder_meant_raw_or_in_zstandard() {
    let mut src = source(40, 36, true);
    for y in 0..8 {
        for x in 0..8 {
            src.put_pixel(x, y, Rgba([200, 30, 90, 255]));
        }
    }
    for zstd in [false, true] {
        let e = uastc::encode(&src, zstd);
        let decoded = decode_ktx2(&e.file).unwrap();
        assert_eq!(decoded, e.meant, "zstd {zstd}");
        let (mean, max) = mean_and_max_error(&decoded, &src);
        assert!(mean < 4.0 && max < 32, "mean {mean}, max {max}");
    }
}

#[test]
fn broken_or_unsupported_files_are_errors_not_panics() {
    let good = uastc::encode(&source(16, 16, false), false).file;
    assert!(decode_ktx2(&good[..60]).is_err());
    assert!(decode_ktx2(&good[..good.len() - 7]).is_err());
    let mut wrong_id = good.clone();
    wrong_id[1] = b'X';
    assert!(decode_ktx2(&wrong_id).is_err());
    let mut astc_model = good.clone();
    astc_model[104 + 12] = 162;
    let err = decode_ktx2(&astc_model).unwrap_err();
    assert!(err.contains("162"), "{err}");
    let mut deflate = good.clone();
    deflate[44..48].copy_from_slice(&3u32.to_le_bytes());
    assert!(decode_ktx2(&deflate).is_err());
    let e = etc1s::encode(&source(16, 16, false), false).file;
    for cut in [100, 140, e.len() - 3] {
        assert!(decode_ktx2(&e[..cut]).is_err(), "cut at {cut}");
    }
    let mut noise = e.clone();
    let start = noise.len() - 40;
    for (i, b) in noise[start..].iter_mut().enumerate() {
        *b ^= (i as u8).wrapping_mul(37) | 1;
    }
    let _ = decode_ktx2(&noise);
}

#[test]
fn a_texture_too_large_to_hold_is_an_error_before_anything_is_allocated() {
    let good = uastc::encode(&source(16, 16, false), false).file;
    let sized = |w: u32, h: u32| {
        let mut f = good.clone();
        f[20..24].copy_from_slice(&w.to_le_bytes());
        f[24..28].copy_from_slice(&h.to_le_bytes());
        f
    };
    for (w, h) in [(u32::MAX, u32::MAX), (1 << 20, 1 << 20), (65_536, 4)] {
        let err = decode_ktx2(&sized(w, h)).unwrap_err();
        assert!(err.contains(&format!("{w}×{h}")), "{err}");
    }
    let tight = Ktx2Settings { max_side: 8 };
    let err = decode_ktx2_with(&good, &tight).unwrap_err();
    assert!(err.contains("8"), "{err}");
    assert!(decode_ktx2_with(&good, &Ktx2Settings { max_side: 16 }).is_ok());
    assert_eq!(Ktx2Settings::default().max_side, 16_384);
}

fn basisu_kit(name: &str, ktx2: &[u8], fallback: Option<&[u8]>) -> std::path::PathBuf {
    let mut bin = ktx2.to_vec();
    bin.resize(bin.len().next_multiple_of(4), 0);
    let png_at = bin.len();
    if let Some(png) = fallback {
        bin.extend(png);
    }
    let (views, images, source) = match fallback {
        Some(png) => (
            format!(
                r#"[{{ "buffer": 0, "byteLength": {} }}, {{ "buffer": 0, "byteOffset": {png_at}, "byteLength": {} }}]"#,
                ktx2.len(),
                png.len()
            ),
            r#"[{ "bufferView": 0, "mimeType": "image/ktx2" }, { "bufferView": 1, "mimeType": "image/png" }]"#,
            r#""source": 1,"#,
        ),
        None => (
            format!(r#"[{{ "buffer": 0, "byteLength": {} }}]"#, ktx2.len()),
            r#"[{ "bufferView": 0, "mimeType": "image/ktx2" }]"#,
            "",
        ),
    };
    let json = format!(
        r#"{{
        "asset": {{ "version": "2.0" }},
        "extensionsUsed": ["KHR_texture_basisu"],
        "extensionsRequired": ["KHR_texture_basisu"],
        "buffers": [{{ "byteLength": {length} }}],
        "bufferViews": {views},
        "images": {images},
        "textures": [{{ {source} "extensions": {{ "KHR_texture_basisu": {{ "source": 0 }} }} }}],
        "materials": [{{
            "name": "pele",
            "pbrMetallicRoughness": {{ "baseColorTexture": {{ "index": 0 }} }}
        }}],
        "scenes": [{{ "nodes": [] }}]
    }}"#,
        length = bin.len()
    );
    write_glb(name, &json, &bin)
}

/// Mip 0 of an ASTC 4×4 texture, decoded.
fn astc_mip0(t: &sariema_format::Texture) -> RgbaImage {
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
    RgbaImage::from_raw(t.width, t.height, texels).unwrap()
}

#[test]
fn a_kit_with_khr_texture_basisu_stores_its_texture_as_rgba8_and_astc_near_the_source() {
    let src = source(64, 64, false);
    for (name, file) in [
        ("basisu_etc1s.glb", etc1s::encode(&src, false).file),
        ("basisu_uastc.glb", uastc::encode(&src, true).file),
    ] {
        let kit = load_kit(&basisu_kit(name, &file, None)).unwrap();
        assert_eq!(kit.materials[0].base_color_texture, Some(0), "{name}");
        let image = &kit.images[0].rgba;
        assert_eq!(image, &decode_ktx2(&file).unwrap(), "{name}");
        let rgba = encode_texture("pele", image, TextureMode::Rgba8).unwrap();
        assert_eq!(rgba.format, TextureFormat::Rgba8Srgb);
        assert_eq!(rgba.mips[0], image.as_raw().as_slice());
        let astc = encode_texture("pele", image, TextureMode::Astc4x4).unwrap();
        assert_eq!(astc.format, TextureFormat::Astc4x4Srgb);
        let (mean, max) = mean_and_max_error(&astc_mip0(&astc), &src);
        assert!(mean < 4.0 && max < 48, "{name}: mean {mean}, max {max}");
    }
}

#[test]
fn a_png_fallback_beside_the_basisu_image_is_used_as_it_is_lossless() {
    let src = source(16, 16, false);
    let mut png = Vec::new();
    src.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let ktx2 = uastc::encode(&src, false).file;
    let kit = load_kit(&basisu_kit("basisu_fallback.glb", &ktx2, Some(&png))).unwrap();
    assert_eq!(kit.materials[0].base_color_texture, Some(1));
    assert_eq!(kit.images[1].rgba, src);
}

/// Every `X.ktx2` in `$SARIEMA_KTX2_DIR` decodes to the RGBA8 texels in
/// `X.rgba` beside it, as the reference transcoder wrote them: a check
/// against real files kept outside the repository.
#[test]
#[ignore = "needs SARIEMA_KTX2_DIR with .ktx2 files and their reference .rgba"]
fn files_decode_exactly_as_the_reference_transcoder() {
    let dir = std::env::var("SARIEMA_KTX2_DIR").expect("SARIEMA_KTX2_DIR");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "ktx2") {
            continue;
        }
        let decoded = decode_ktx2(&std::fs::read(&path).unwrap()).unwrap();
        let reference = std::fs::read(path.with_extension("rgba")).unwrap();
        let differing = (decoded.as_raw().chunks(4))
            .zip(reference.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            differing,
            0,
            "{}: {differing} texels differ",
            path.display()
        );
        checked += 1;
    }
    assert!(checked > 0, "no .ktx2 in {dir}");
}
