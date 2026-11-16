// SPDX-License-Identifier: Apache-2.0

//! Sample-rate conversion with a Kaiser-windowed sinc, at load time for
//! decoded sounds and on the decoding thread for streams. The mixer itself
//! only ever reads audio at its own rate.
//!
//! Positions step by the exact ratio of the two rates in integers, so a
//! stream of any length never drifts.

use std::f64::consts::PI;

/// Fractional positions are quantised to this many steps between two
/// source frames, and the coefficients interpolated linearly between steps.
const PHASES: usize = 256;
const KAISER_BETA: f64 = 8.0;
/// Fraction of the lower Nyquist frequency the filter passes.
const PASSBAND: f64 = 0.95;

pub(crate) struct Kernel {
    half: usize,
    /// `PHASES + 1` rows of `2 * half` taps; row `j` is for fraction `j / PHASES`.
    table: Vec<f32>,
}

impl Kernel {
    pub fn new(half_taps: u32, src_rate: u32, out_rate: u32) -> Kernel {
        let half = half_taps.max(1) as usize;
        let cutoff = PASSBAND * (f64::from(out_rate) / f64::from(src_rate)).min(1.0);
        let taps = 2 * half;
        let mut table = Vec::with_capacity((PHASES + 1) * taps);
        let mut row = Vec::with_capacity(taps);
        for j in 0..=PHASES {
            let frac = j as f64 / PHASES as f64;
            row.clear();
            row.extend((0..taps).map(|k| {
                let d = k as f64 - half as f64 + 1.0 - frac;
                cutoff * sinc(cutoff * d) * kaiser(d / half as f64)
            }));
            let sum: f64 = row.iter().sum();
            table.extend(row.iter().map(|c| (c / sum) as f32));
        }
        Kernel { half, table }
    }

    pub fn half(&self) -> usize {
        self.half
    }

    /// One output sample at `index + num / den` of the source, reading
    /// source frame `i` through `x`.
    fn sample(&self, index: i64, num: u64, den: u64, x: impl Fn(i64) -> f32) -> f32 {
        let taps = 2 * self.half;
        let scaled = num as f64 * PHASES as f64 / den as f64;
        let j = (scaled as usize).min(PHASES - 1);
        let t = (scaled - j as f64) as f32;
        let (a, b) = (
            &self.table[j * taps..][..taps],
            &self.table[(j + 1) * taps..][..taps],
        );
        let first = index - self.half as i64 + 1;
        let mut acc = 0.0;
        for k in 0..taps {
            let c = a[k] + (b[k] - a[k]) * t;
            acc += c * x(first + k as i64);
        }
        acc
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

fn kaiser(x: f64) -> f64 {
    if x.abs() > 1.0 {
        return 0.0;
    }
    bessel_i0(KAISER_BETA * (1.0 - x * x).sqrt()) / bessel_i0(KAISER_BETA)
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..50 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
    }
    sum
}

/// Interleaved `channels`-channel audio at `src_rate` brought to `out_rate`.
/// A loop region `[start, end)` in source frames is converted as a circle,
/// so its seam stays smooth, and its new bounds are returned with it.
pub(crate) fn resample(
    src: &[f32],
    channels: usize,
    src_rate: u32,
    out_rate: u32,
    half_taps: u32,
    loop_points: Option<(u64, u64)>,
) -> (Vec<f32>, Option<(u64, u64)>) {
    if src_rate == out_rate {
        return (src.to_vec(), loop_points);
    }
    let frames = (src.len() / channels) as u64;
    let (s, o) = (u64::from(src_rate), u64::from(out_rate));
    let to_out = |n: u64| (n * o + s / 2) / s;
    let kernel = Kernel::new(half_taps, src_rate, out_rate);
    let direct = |c: usize| {
        move |i: i64| {
            if (0..frames as i64).contains(&i) {
                src[i as usize * channels + c]
            } else {
                0.0
            }
        }
    };
    let out_frames = match loop_points {
        None => (frames * o).div_ceil(s),
        Some((ls, le)) => to_out(ls) + to_out(le - ls).max(1) + ((frames - le) * o).div_ceil(s),
    };
    let mut out = Vec::with_capacity(out_frames as usize * channels);
    let mut run =
        |from: u64, count: u64, step_num: u64, step_den: u64, wrap: Option<(u64, u64)>| {
            for n in 0..count {
                let pos = n * step_num;
                let index = (from + pos / step_den) as i64;
                let rem = pos % step_den;
                for c in 0..channels {
                    let value = match wrap {
                        Some((ls, le)) => {
                            let len = (le - ls) as i64;
                            let read = direct(c);
                            kernel.sample(index, rem, step_den, |i| {
                                read(ls as i64 + (i - ls as i64).rem_euclid(len))
                            })
                        }
                        None => kernel.sample(index, rem, step_den, direct(c)),
                    };
                    out.push(value);
                }
            }
        };
    match loop_points {
        None => {
            run(0, (frames * o).div_ceil(s), s, o, None);
            (out, None)
        }
        Some((ls, le)) => {
            let (ls_out, len_out) = (to_out(ls), to_out(le - ls).max(1));
            run(0, ls_out, s, o, None);
            run(ls, len_out, le - ls, len_out, Some((ls, le)));
            run(le, ((frames - le) * o).div_ceil(s), s, o, None);
            (out, Some((ls_out, ls_out + len_out)))
        }
    }
}

/// Converts an endless stereo stream chunk by chunk; output positions step
/// by exactly `src_rate / out_rate` source frames.
pub(crate) struct StreamResampler {
    kernel: Kernel,
    src_rate: u64,
    out_rate: u64,
    /// Stereo source frames; frame 0 here is source frame `base`.
    buf: Vec<f32>,
    base: i64,
    index: i64,
    rem: u64,
}

impl StreamResampler {
    pub fn new(half_taps: u32, src_rate: u32, out_rate: u32) -> StreamResampler {
        let kernel = Kernel::new(half_taps, src_rate, out_rate);
        let half = kernel.half() as i64;
        StreamResampler {
            // Silence before the first frame, so the first output is centred on it.
            buf: vec![0.0; 2 * (half as usize - 1)],
            base: -(half - 1),
            kernel,
            src_rate: u64::from(src_rate),
            out_rate: u64::from(out_rate),
            index: 0,
            rem: 0,
        }
    }

    /// Adds stereo source frames and appends every output frame they make
    /// computable to `out`.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.buf.extend_from_slice(input);
        let half = self.kernel.half() as i64;
        let buffered = self.base + (self.buf.len() / 2) as i64;
        while self.index + half < buffered {
            for c in 0..2 {
                let (buf, base) = (&self.buf, self.base);
                let read = |i: i64| buf[(i - base) as usize * 2 + c];
                out.push(
                    self.kernel
                        .sample(self.index, self.rem, self.out_rate, read),
                );
            }
            self.rem += self.src_rate;
            self.index += (self.rem / self.out_rate) as i64;
            self.rem %= self.out_rate;
        }
        let keep_from = self.index - half + 1;
        if keep_from > self.base {
            self.buf.drain(..(keep_from - self.base) as usize * 2);
            self.base = keep_from;
        }
    }

    /// Pads the end with silence so the last source frames come out.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        let pad = vec![0.0; 2 * self.kernel.half()];
        self.push(&pad, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| (std::f32::consts::TAU * freq * n as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn same_rate_is_untouched() {
        let x = sine(440.0, 48_000, 100);
        assert_eq!(resample(&x, 1, 48_000, 48_000, 16, None).0, x);
    }

    #[test]
    fn converted_sine_matches_the_ideal_one() {
        let x = sine(1000.0, 44_100, 44_100);
        let (y, _) = resample(&x, 1, 44_100, 48_000, 16, None);
        assert_eq!(y.len(), 48_000);
        let ideal = sine(1000.0, 48_000, 48_000);
        let worst = (100..47_900)
            .map(|n| (y[n] - ideal[n]).abs())
            .fold(0.0, f32::max);
        assert!(worst < 2e-3, "{worst}");
    }

    #[test]
    fn stream_matches_the_whole_buffer_conversion() {
        let mono = sine(330.0, 44_100, 10_000);
        let stereo: Vec<f32> = mono.iter().flat_map(|&v| [v, -v]).collect();
        let (whole, _) = resample(&stereo, 2, 44_100, 48_000, 16, None);
        let mut stream = StreamResampler::new(16, 44_100, 48_000);
        let mut out = Vec::new();
        for chunk in stereo.chunks(2 * 777) {
            stream.push(chunk, &mut out);
        }
        stream.finish(&mut out);
        let n = out.len().min(whole.len());
        assert!(n > 2 * 10_800);
        let worst = (0..n)
            .map(|i| (out[i] - whole[i]).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-6, "{worst}");
    }
}
