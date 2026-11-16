// SPDX-License-Identifier: Apache-2.0

//! The bit stream of Basis Universal's ETC1S data: bits least significant
//! first, canonical Huffman codes read most significant bit first, and the
//! code-length coding of its Huffman tables.

pub struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
    buf: u64,
    len: u32,
}

impl<'a> Bits<'a> {
    pub fn new(bytes: &'a [u8]) -> Bits<'a> {
        Bits {
            bytes,
            at: 0,
            buf: 0,
            len: 0,
        }
    }

    /// `n` bits, at most 32; past the end the stream reads zeros, which
    /// [`Bits::check`] then reports.
    pub fn get(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        while self.len < n {
            let byte = self.bytes.get(self.at).copied().unwrap_or(0);
            self.at += 1;
            self.buf |= u64::from(byte) << self.len;
            self.len += 8;
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.len -= n;
        v
    }

    /// A variable-length number in chunks of `chunk` bits, each followed
    /// by a bit saying whether another chunk follows.
    pub fn vlc(&mut self, chunk: u32) -> u32 {
        let (mut v, mut shift) = (0u32, 0);
        loop {
            let s = self.get(chunk + 1);
            v |= (s & ((1 << chunk) - 1)).checked_shl(shift).unwrap_or(0);
            shift += chunk;
            if s & (1 << chunk) == 0 || shift >= 32 {
                return v;
            }
        }
    }

    /// An error if more bits were read than the stream holds.
    pub fn check(&self, what: &str) -> Result<(), String> {
        let read = self.at as u64 * 8 - u64::from(self.len);
        if read > self.bytes.len() as u64 * 8 {
            return Err(format!("{what} ends early"));
        }
        Ok(())
    }
}

/// A canonical Huffman code: symbols ordered by code size, then by value.
pub struct Huffman {
    /// Codes of each size, 1 to 31.
    counts: [u32; 32],
    symbols: Vec<u16>,
}

impl Huffman {
    /// The code with these sizes, 0 for a symbol without one. It must be
    /// complete, or have one symbol alone.
    pub fn new(sizes: &[u8]) -> Result<Huffman, String> {
        let mut counts = [0u32; 32];
        for &s in sizes {
            if s > 31 {
                return Err(format!("a Huffman code of {s} bits"));
            }
            counts[s as usize] += 1;
        }
        counts[0] = 0;
        let used: u32 = counts.iter().sum();
        let kraft: u64 = (1..32).map(|l| u64::from(counts[l]) << (31 - l)).sum();
        if used != 1 && kraft != 1 << 31 {
            return Err("a Huffman table that is not a complete code".into());
        }
        let mut symbols: Vec<(u8, u16)> = (sizes.iter().enumerate())
            .filter(|&(_, &s)| s > 0)
            .map(|(i, &s)| (s, i as u16))
            .collect();
        symbols.sort_unstable();
        Ok(Huffman {
            counts,
            symbols: symbols.into_iter().map(|(_, sym)| sym).collect(),
        })
    }

    pub fn decode(&self, bits: &mut Bits) -> Result<u32, String> {
        let (mut code, mut first, mut index) = (0u32, 0u32, 0u32);
        for len in 1..32 {
            code |= bits.get(1);
            let count = self.counts[len];
            if code.wrapping_sub(first) < count {
                return Ok(u32::from(self.symbols[(index + code - first) as usize]));
            }
            index += count;
            first = (first + count).wrapping_shl(1);
            code = code.wrapping_shl(1);
        }
        Err("a bit pattern that is no Huffman code".into())
    }
}

const SORTED_CODE_LENGTH_CODES: [usize; 21] = [
    17, 18, 19, 20, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15, 16,
];
const MAX_SYMBOLS: u32 = 1 << 14;

/// A Huffman table; `None` when it has no symbols.
pub fn table(bits: &mut Bits) -> Result<Option<Huffman>, String> {
    let total = bits.get(14);
    if total == 0 {
        return Ok(None);
    }
    if total > MAX_SYMBOLS {
        return Err(format!("a Huffman table of {total} symbols"));
    }
    let codes = bits.get(5) as usize;
    if !(1..=SORTED_CODE_LENGTH_CODES.len()).contains(&codes) {
        return Err(format!("a Huffman table with {codes} code-length codes"));
    }
    let mut lengths = [0u8; 21];
    for &c in &SORTED_CODE_LENGTH_CODES[..codes] {
        lengths[c] = bits.get(3) as u8;
    }
    let lengths = Huffman::new(&lengths)?;
    let total = total as usize;
    let mut sizes = vec![0u8; total];
    let mut at = 0;
    while at < total {
        match lengths.decode(bits)? {
            c @ 0..=16 => {
                sizes[at] = c as u8;
                at += 1;
            }
            17 => at += bits.get(3) as usize + 3,
            18 => at += bits.get(7) as usize + 11,
            c => {
                let repeat = if c == 19 {
                    bits.get(2) + 3
                } else {
                    bits.get(7) + 7
                };
                let previous = at.checked_sub(1).map_or(0, |p| sizes[p]);
                if previous == 0 || at + repeat as usize > total {
                    return Err("a Huffman table repeats a size it does not have".into());
                }
                sizes[at..at + repeat as usize].fill(previous);
                at += repeat as usize;
            }
        }
    }
    if at != total {
        return Err("a Huffman table's zero run passes its end".into());
    }
    Huffman::new(&sizes).map(Some)
}
