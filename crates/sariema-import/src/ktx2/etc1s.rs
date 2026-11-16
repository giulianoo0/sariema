// SPDX-License-Identifier: Apache-2.0

//! ETC1S in BasisLZ: a codebook of endpoints (a 5-bit colour and an
//! intensity table) and selectors (2 bits a texel), shared by every slice,
//! and slices whose blocks name one of each, predicted from their
//! neighbours, with recent selectors kept in a history and runs of them.

use super::bits::{Bits, Huffman, table};

const INTENSITY: [[i32; 4]; 8] = [
    [-8, -2, 2, 8],
    [-17, -5, 5, 17],
    [-29, -9, 9, 29],
    [-42, -13, 13, 42],
    [-60, -18, 18, 60],
    [-80, -24, 24, 80],
    [-106, -33, 33, 106],
    [-183, -47, 47, 183],
];

/// A colour channel's previous value up to which each of the three delta
/// tables codes the next.
const COLOR5_TABLE_LIMITS: [u8; 2] = [9, 21];
const PREDICTIONS_REPEAT: u32 = 256;
const PREDICTIONS_REPEAT_MIN: u32 = 3;
const RUN_LONG: u32 = 63;
const RUN_MIN: u32 = 3;

#[derive(Clone, Copy)]
struct Endpoint {
    color5: [u8; 3],
    intensity: u8,
}

impl Endpoint {
    /// The block's four colours, by selector.
    fn colors(self) -> [[u8; 3]; 4] {
        let base = self.color5.map(|c| i32::from((c << 3) | (c >> 2)));
        INTENSITY[self.intensity as usize].map(|m| base.map(|b| (b + m).clamp(0, 255) as u8))
    }
}

/// Selectors of a block, a row a byte, two bits a texel, the left one lowest.
type Selector = [u8; 4];

pub struct Codebook {
    endpoints: Vec<Endpoint>,
    selectors: Vec<Selector>,
    predictions: Huffman,
    endpoint_delta: Huffman,
    selector: Huffman,
    selector_run: Huffman,
    history: usize,
}

fn required(t: Option<Huffman>, what: &str) -> Result<Huffman, String> {
    t.ok_or_else(|| format!("ETC1S {what} table is empty"))
}

impl Codebook {
    pub fn read(
        (endpoint_data, endpoints): (&[u8], usize),
        (selector_data, selectors): (&[u8], usize),
        tables: &[u8],
    ) -> Result<Codebook, String> {
        if endpoints == 0 || selectors == 0 {
            return Err("ETC1S codebook without endpoints or selectors".into());
        }
        let mut bits = Bits::new(tables);
        let predictions = required(table(&mut bits)?, "endpoint prediction")?;
        let endpoint_delta = required(table(&mut bits)?, "endpoint delta")?;
        let selector = required(table(&mut bits)?, "selector")?;
        let selector_run = required(table(&mut bits)?, "selector run")?;
        let history = bits.get(13) as usize;
        if history == 0 {
            return Err("ETC1S selector history of 0".into());
        }
        bits.check("ETC1S tables")?;
        Ok(Codebook {
            endpoints: read_endpoints(endpoint_data, endpoints)?,
            selectors: read_selectors(selector_data, selectors)?,
            predictions,
            endpoint_delta,
            selector,
            selector_run,
            history,
        })
    }

    /// Decodes a slice of `blocks` blocks across and down, giving each
    /// block's place, its four colours and its selectors to `put`.
    pub fn decode(
        &self,
        data: &[u8],
        (across, down): (usize, usize),
        mut put: impl FnMut(usize, usize, &[[u8; 3]; 4], &Selector),
    ) -> Result<(), String> {
        let mut bits = Bits::new(data);
        let (endpoints, selectors) = (self.endpoints.len(), self.selectors.len());
        let first_history = selectors;
        let run_symbol = first_history + self.history;
        let total = (across * down) as u32;
        // By block of the row above (the other half) and of this one: its
        // endpoint, and for an even row the predictions of the odd row under.
        let mut rows = vec![[(0usize, 0u8); 2]; across];
        let mut history = History::new(self.history);
        let (mut predictions, mut last_predictions, mut repeat) = (0u32, 0u32, 0u32);
        let (mut previous_endpoint, mut run) = (0usize, 0u32);
        for by in 0..down {
            let row = by & 1;
            for bx in 0..across {
                if bx & 1 == 0 {
                    if row == 0 {
                        if repeat > 0 {
                            repeat -= 1;
                            predictions = last_predictions;
                        } else {
                            predictions = self.predictions.decode(&mut bits)?;
                            if predictions == PREDICTIONS_REPEAT {
                                repeat = bits.vlc(4) + PREDICTIONS_REPEAT_MIN - 1;
                                predictions = last_predictions;
                            } else {
                                last_predictions = predictions;
                            }
                        }
                        rows[bx][row ^ 1].1 = (predictions >> 4) as u8;
                    } else {
                        predictions = u32::from(rows[bx][row].1);
                    }
                }
                let corrupt = || format!("ETC1S block {bx}, {by} predicts from outside the slice");
                let endpoint = match predictions & 3 {
                    0 if bx > 0 => previous_endpoint,
                    1 if by > 0 => rows[bx][row ^ 1].0,
                    2 if bx > 0 && by > 0 => rows[bx - 1][row ^ 1].0,
                    3 => {
                        let e = self.endpoint_delta.decode(&mut bits)? as usize + previous_endpoint;
                        if e >= endpoints { e - endpoints } else { e }
                    }
                    _ => return Err(corrupt()),
                };
                predictions >>= 2;
                rows[bx][row].0 = endpoint;
                previous_endpoint = endpoint;

                let symbol = if run > 0 {
                    run -= 1;
                    first_history
                } else {
                    let s = self.selector.decode(&mut bits)? as usize;
                    if s == run_symbol {
                        let r = self.selector_run.decode(&mut bits)?;
                        run = if r == RUN_LONG { bits.vlc(7) } else { r } + RUN_MIN;
                        if run > total {
                            return Err("ETC1S selector run longer than the slice".into());
                        }
                        run -= 1;
                        first_history
                    } else {
                        s
                    }
                };
                let selector = if symbol >= first_history {
                    let h = symbol - first_history;
                    if h >= self.history {
                        return Err("ETC1S selector history index past its end".into());
                    }
                    history.take(h)
                } else {
                    history.add(symbol);
                    symbol
                };
                if endpoint >= endpoints || selector >= selectors {
                    return Err(format!("ETC1S block {bx}, {by} names no codebook entry"));
                }
                put(
                    bx,
                    by,
                    &self.endpoints[endpoint].colors(),
                    &self.selectors[selector],
                );
            }
        }
        bits.check("ETC1S slice")
    }
}

/// Recently used selectors: a new one goes in at a rover over the back
/// half, and one used again swaps halfway to the front.
struct History {
    values: Vec<usize>,
    rover: usize,
}

impl History {
    fn new(size: usize) -> History {
        History {
            values: vec![0; size],
            rover: size / 2,
        }
    }

    fn add(&mut self, value: usize) {
        self.values[self.rover] = value;
        self.rover += 1;
        if self.rover == self.values.len() {
            self.rover = self.values.len() / 2;
        }
    }

    fn take(&mut self, index: usize) -> usize {
        let value = self.values[index];
        if index > 0 {
            self.values.swap(index / 2, index);
        }
        value
    }
}

fn read_endpoints(data: &[u8], count: usize) -> Result<Vec<Endpoint>, String> {
    let mut bits = Bits::new(data);
    let mut deltas = Vec::with_capacity(3);
    for _ in 0..3 {
        deltas.push(required(table(&mut bits)?, "colour delta")?);
    }
    let intensity_delta = required(table(&mut bits)?, "intensity delta")?;
    let grey = bits.get(1) == 1;
    let (mut previous, mut previous_intensity) = ([16u8; 3], 0u8);
    let mut endpoints = Vec::with_capacity(count);
    for _ in 0..count {
        let intensity =
            (intensity_delta.decode(&mut bits)? as u8).wrapping_add(previous_intensity) & 7;
        previous_intensity = intensity;
        let mut color5 = [0u8; 3];
        for c in 0..if grey { 1 } else { 3 } {
            let t = COLOR5_TABLE_LIMITS
                .iter()
                .filter(|&&l| previous[c] > l)
                .count();
            let delta = deltas[t].decode(&mut bits)? as u8;
            color5[c] = previous[c].wrapping_add(delta) & 31;
            previous[c] = color5[c];
        }
        if grey {
            color5 = [color5[0]; 3];
        }
        endpoints.push(Endpoint { color5, intensity });
    }
    bits.check("ETC1S endpoints")?;
    Ok(endpoints)
}

fn read_selectors(data: &[u8], count: usize) -> Result<Vec<Selector>, String> {
    let mut bits = Bits::new(data);
    if bits.get(1) == 1 || bits.get(1) == 1 {
        return Err("ETC1S with a global selector codebook".into());
    }
    let raw = bits.get(1) == 1;
    let mut selectors = Vec::with_capacity(count);
    if raw {
        for _ in 0..count {
            selectors.push([(); 4].map(|_| bits.get(8) as u8));
        }
    } else {
        let delta = table(&mut bits)?;
        let mut previous = [0u8; 4];
        for i in 0..count {
            if i == 0 {
                previous = [(); 4].map(|_| bits.get(8) as u8);
            } else {
                let delta = delta
                    .as_ref()
                    .ok_or("ETC1S selector delta table is empty")?;
                for p in &mut previous {
                    *p ^= delta.decode(&mut bits)? as u8;
                }
            }
            selectors.push(previous);
        }
    }
    bits.check("ETC1S selectors")?;
    Ok(selectors)
}
