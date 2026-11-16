// SPDX-License-Identifier: Apache-2.0

//! UASTC LDR 4×4: 128-bit blocks in one of 19 modes, each a subset of
//! ASTC 4×4 (direct RGB, RGBA or luminance-alpha endpoints, one to three
//! partitions from a fixed set, one or two weight planes) or a solid colour.
//! The transcoding hints for other formats are skipped.

const MODES: usize = 19;
const SOLID: usize = 8;

/// Each mode's prefix code, read from bit 0, and its length.
const MODE_CODES: [(u32, u32); MODES] = [
    (0x1, 4),
    (0x35, 6),
    (0x1D, 5),
    (0x3, 5),
    (0x13, 5),
    (0xB, 5),
    (0x1B, 5),
    (0x7, 5),
    (0x17, 5),
    (0xF, 5),
    (0x2, 3),
    (0x0, 2),
    (0x6, 3),
    (0x1F, 5),
    (0xD, 5),
    (0x5, 7),
    (0x15, 6),
    (0x25, 6),
    (0x9, 4),
];

/// The mode of each 7-bit prefix; 19 is reserved.
const MODE_OF_PREFIX: [u8; 128] = [
    11, 0, 10, 3, 11, 15, 12, 7, 11, 18, 10, 5, 11, 14, 12, 9, 11, 0, 10, 4, 11, 16, 12, 8, 11, 18,
    10, 6, 11, 2, 12, 13, 11, 0, 10, 3, 11, 17, 12, 7, 11, 18, 10, 5, 11, 14, 12, 9, 11, 0, 10, 4,
    11, 1, 12, 8, 11, 18, 10, 6, 11, 2, 12, 13, 11, 0, 10, 3, 11, 19, 12, 7, 11, 18, 10, 5, 11, 14,
    12, 9, 11, 0, 10, 4, 11, 16, 12, 8, 11, 18, 10, 6, 11, 2, 12, 13, 11, 0, 10, 3, 11, 17, 12, 7,
    11, 18, 10, 5, 11, 14, 12, 9, 11, 0, 10, 4, 11, 1, 12, 8, 11, 18, 10, 6, 11, 2, 12, 13,
];

const WEIGHT_BITS: [u32; MODES] = [4, 2, 3, 2, 2, 3, 2, 2, 0, 2, 4, 2, 3, 1, 2, 4, 2, 2, 5];
const ENDPOINT_RANGE: [usize; MODES] = [
    19, 20, 8, 7, 12, 20, 18, 12, 0, 8, 13, 13, 19, 20, 20, 20, 20, 20, 11,
];
const COMPONENTS: [usize; MODES] = [3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 2, 2, 2, 3];
const HINT_BITS: [u32; MODES] = [
    15, 15, 15, 15, 15, 15, 15, 15, 0, 23, 17, 17, 17, 23, 23, 23, 23, 23, 15,
];

/// Two-partition patterns shared by ASTC and BC7, by UASTC's pattern index.
const PATTERNS2: [[u8; 16]; 30] = [
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1],
    [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1],
    [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0],
    [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 0, 0],
    [0, 0, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1],
    [1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 0, 0],
    [1, 1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0],
    [1, 0, 0, 0, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 0, 0, 0, 1],
    [0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0],
    [0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 0, 0, 1, 1],
    [1, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0],
    [0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0],
    [1, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0, 0, 1, 1],
    [0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0],
    [1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1],
    [1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0],
    [1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0],
    [1, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0, 0],
];

const PATTERNS3: [[u8; 16]; 11] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2],
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 2, 2, 2, 2],
    [1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 2],
    [1, 1, 1, 1, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2, 0],
    [0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2],
    [0, 2, 1, 1, 0, 2, 1, 1, 0, 2, 1, 1, 0, 2, 1, 1],
    [2, 0, 0, 0, 2, 0, 0, 0, 2, 1, 1, 1, 2, 1, 1, 1],
    [2, 0, 1, 2, 2, 0, 1, 2, 2, 0, 1, 2, 2, 0, 1, 2],
    [1, 1, 1, 1, 0, 0, 0, 0, 2, 2, 2, 2, 1, 1, 1, 1],
    [0, 0, 2, 2, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 2, 2],
];

/// Mode 7's two-partition patterns, from BC7's three-partition ones.
const PATTERNS2_OF_3: [[u8; 16]; 19] = [
    [0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0],
    [1, 1, 0, 0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1],
    [0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0],
    [0, 0, 0, 1, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1],
    [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0],
    [0, 1, 1, 1, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 1, 1, 1, 0],
    [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0],
    [0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 0],
    [1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 0, 0, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 0],
    [0, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 0],
    [1, 1, 1, 1, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
];

/// The first texel of each partition, whose weight is stored a bit short.
const ANCHORS2: [[u8; 2]; 30] = [
    [0, 2],
    [0, 3],
    [1, 0],
    [0, 3],
    [7, 0],
    [0, 2],
    [3, 0],
    [7, 0],
    [0, 11],
    [2, 0],
    [0, 7],
    [11, 0],
    [3, 0],
    [8, 0],
    [0, 4],
    [12, 0],
    [1, 0],
    [8, 0],
    [0, 1],
    [0, 2],
    [0, 4],
    [8, 0],
    [1, 0],
    [0, 2],
    [4, 0],
    [0, 1],
    [4, 0],
    [1, 0],
    [4, 0],
    [1, 0],
];

const ANCHORS3: [[u8; 3]; 11] = [
    [0, 8, 10],
    [8, 0, 12],
    [4, 0, 12],
    [8, 0, 4],
    [3, 0, 2],
    [0, 1, 3],
    [0, 2, 1],
    [1, 9, 0],
    [1, 2, 0],
    [4, 0, 8],
    [0, 6, 2],
];

const ANCHORS2_OF_3: [[u8; 2]; 19] = [
    [0, 4],
    [0, 2],
    [2, 0],
    [0, 7],
    [8, 0],
    [0, 1],
    [0, 3],
    [0, 1],
    [2, 0],
    [0, 1],
    [0, 8],
    [2, 0],
    [0, 1],
    [0, 7],
    [12, 0],
    [2, 0],
    [9, 0],
    [0, 2],
    [4, 0],
];

/// Bits, trits and quints of each ASTC endpoint range.
const BISE: [(u32, u32, u32); 21] = [
    (1, 0, 0),
    (0, 1, 0),
    (2, 0, 0),
    (0, 0, 1),
    (1, 1, 0),
    (3, 0, 0),
    (1, 0, 1),
    (2, 1, 0),
    (4, 0, 0),
    (2, 0, 1),
    (3, 1, 0),
    (5, 0, 0),
    (3, 0, 1),
    (4, 1, 0),
    (6, 0, 0),
    (4, 0, 1),
    (5, 1, 0),
    (7, 0, 0),
    (5, 0, 1),
    (6, 1, 0),
    (8, 0, 0),
];

/// The ASTC specification's unquantization of trit and quint ranges: which
/// of the value's bits make up B (`a` its lowest, `0` none), and C.
const UNQUANTIZE: [(&[u8; 9], u32); 21] = [
    (b"000000000", 0),
    (b"000000000", 0),
    (b"000000000", 0),
    (b"000000000", 0),
    (b"000000000", 204),
    (b"000000000", 0),
    (b"000000000", 113),
    (b"b000b0bb0", 93),
    (b"000000000", 0),
    (b"b0000bb00", 54),
    (b"cb000cbcb", 44),
    (b"000000000", 0),
    (b"cb0000cbc", 26),
    (b"dcb000dcb", 22),
    (b"000000000", 0),
    (b"dcb0000dc", 13),
    (b"edcb000ed", 11),
    (b"000000000", 0),
    (b"edcb0000e", 6),
    (b"fedcb000f", 5),
    (b"000000000", 0),
];

const WEIGHTS: [&[u32]; 6] = [
    &[0],
    &[0, 64],
    &[0, 21, 43, 64],
    &[0, 9, 18, 27, 37, 46, 55, 64],
    &[0, 4, 8, 12, 17, 21, 25, 29, 35, 39, 43, 47, 52, 56, 60, 64],
    &[
        0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 34, 36, 38, 40, 42, 44, 46, 48,
        50, 52, 54, 56, 58, 60, 62, 64,
    ],
];

/// An endpoint value of `range` as an 8-bit one.
fn unquantize(value: u32, range: usize) -> u32 {
    let (bits, trits, quints) = BISE[range];
    let low = value & ((1 << bits) - 1);
    if trits == 0 && quints == 0 {
        let (mut v, mut left) = (0, 8i32);
        while left > 0 {
            let n = left.min(bits as i32);
            v |= (low >> (bits as i32 - n)) << (left - n);
            left -= n;
        }
        return v;
    }
    let (letters, c) = UNQUANTIZE[range];
    let a = if low & 1 == 1 { 511 } else { 0 };
    let b = letters.iter().fold(0, |b, &l| {
        (b << 1)
            | if l == b'0' {
                0
            } else {
                (low >> (l - b'a')) & 1
            }
    });
    let v = ((value >> bits) * c + b) ^ a;
    (a & 0x80) | (v >> 2)
}

fn interpolate(l: u32, h: u32, w: u32) -> u8 {
    let (l, h) = (l * 257, h * 257);
    ((l * (64 - w) + h * w + 32) >> 6 >> 8) as u8
}

struct Reader {
    block: u128,
    at: u32,
}

impl Reader {
    fn get(&mut self, n: u32) -> Result<u32, String> {
        if self.at + n > 128 {
            return Err("UASTC block reads past its 128 bits".into());
        }
        let v = ((self.block >> self.at) & ((1u128 << n) - 1)) as u32;
        self.at += n;
        Ok(v)
    }
}

/// The 16 texels of a block, row by row.
pub fn decode(block: &[u8; 16]) -> Result<[[u8; 4]; 16], String> {
    let mode = MODE_OF_PREFIX[(block[0] & 127) as usize] as usize;
    if mode >= MODES {
        return Err(format!("UASTC block in reserved mode {mode}"));
    }
    let mut r = Reader {
        block: u128::from_le_bytes(*block),
        at: MODE_CODES[mode].1,
    };
    if mode == SOLID {
        let mut c = [0u8; 4];
        for v in &mut c {
            *v = r.get(8)? as u8;
        }
        return Ok([c; 16]);
    }
    r.at += HINT_BITS[mode];
    let (subsets, pattern) = match mode {
        2 | 4 | 7 | 9 | 16 => (2, r.get(5)? as usize),
        3 => (3, r.get(4)? as usize),
        _ => (1, 0),
    };
    let (pattern, anchors): (&[u8; 16], &[u8]) = match (subsets, mode) {
        (1, _) => (&[0; 16], &[0]),
        (3, _) => (
            PATTERNS3.get(pattern).ok_or("UASTC pattern out of range")?,
            &ANCHORS3[pattern][..],
        ),
        (_, 7) => (
            PATTERNS2_OF_3
                .get(pattern)
                .ok_or("UASTC pattern out of range")?,
            &ANCHORS2_OF_3[pattern][..],
        ),
        _ => (
            PATTERNS2.get(pattern).ok_or("UASTC pattern out of range")?,
            &ANCHORS2[pattern][..],
        ),
    };
    let (planes, ccs) = match mode {
        6 | 11 | 13 => (2, r.get(2)? as usize),
        17 => (2, 3),
        _ => (1, 4),
    };

    let comps = COMPONENTS[mode];
    let range = ENDPOINT_RANGE[mode];
    let (bits, trits, quints) = BISE[range];
    let count = comps * 2 * subsets;
    let (bundle, base) = match (trits, quints) {
        (1, _) => (5, 3),
        (_, 1) => (3, 5),
        _ => (0, 0),
    };
    let mut packed = [0u32; 8];
    let bundles = if bundle > 0 {
        count.div_ceil(bundle)
    } else {
        0
    };
    for (i, p) in packed.iter_mut().enumerate().take(bundles) {
        let mut n = if trits == 1 { 8 } else { 7 };
        if i == bundles - 1 {
            let left = count - (bundles - 1) * bundle;
            n = match (trits, left) {
                (1, 1) => 2,
                (1, 2) => 4,
                (1, 3) => 5,
                (1, 4) => 7,
                (0, 1) => 3,
                (0, 2) => 5,
                _ => n,
            };
        }
        *p = r.get(n)?;
    }
    let mut endpoints = [0u32; 18];
    let (mut accum, mut left, mut next) = (0u32, 0, 0);
    for e in endpoints.iter_mut().take(count) {
        let mut v = r.get(bits)?;
        if bundle > 0 {
            if left == 0 {
                accum = packed[next];
                next += 1;
                left = bundle;
            }
            v |= (accum % base) << bits;
            accum /= base;
            left -= 1;
        }
        *e = unquantize(v, range);
    }

    let weight_bits = WEIGHT_BITS[mode];
    let mut weights = [0u32; 32];
    for (i, w) in weights.iter_mut().enumerate().take(16 * planes) {
        let texel = (i >> (planes - 1)) as u8;
        let short = anchors.contains(&texel);
        *w = r.get(weight_bits - u32::from(short))?;
    }

    let table = WEIGHTS[weight_bits as usize];
    let mut colors = [[[0u8; 4]; 32]; 3];
    for (s, subset) in colors.iter_mut().enumerate().take(subsets) {
        let e = &endpoints[s * comps * 2..];
        let ends: [[u32; 2]; 4] = match comps {
            2 => [[e[0], e[1]], [e[0], e[1]], [e[0], e[1]], [e[2], e[3]]],
            3 => [[e[0], e[1]], [e[2], e[3]], [e[4], e[5]], [255, 255]],
            _ => [[e[0], e[1]], [e[2], e[3]], [e[4], e[5]], [e[6], e[7]]],
        };
        for (l, &w) in table.iter().enumerate() {
            subset[l] = ends.map(|[lo, hi]| interpolate(lo, hi, w));
        }
    }
    Ok(std::array::from_fn(|i| {
        if planes == 1 {
            colors[pattern[i] as usize][weights[i] as usize]
        } else {
            let (a, b) = (weights[i * 2] as usize, weights[i * 2 + 1] as usize);
            std::array::from_fn(|c| colors[0][if c == ccs { b } else { a }][c])
        }
    }))
}
