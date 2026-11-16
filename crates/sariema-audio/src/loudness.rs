// SPDX-License-Identifier: Apache-2.0

//! Integrated loudness after ITU-R BS.1770-4: K-weighting, 400 ms blocks
//! overlapping by 75 %, an absolute gate at −70 LUFS and a relative gate
//! 10 LU under the level of the blocks that pass it. The K-weighting
//! coefficients for any sample rate follow libebur128's `ebur128_init_filter`.

use std::f64::consts::PI;

/// Mono and stereo channels all weigh 1 in BS.1770.
pub struct LoudnessMeter {
    channels: usize,
    hop: usize,
    filters: Vec<[Biquad; 2]>,
    /// Sum over channels of the mean square of each 100 ms hop.
    hops: Vec<f64>,
    acc: f64,
    acc_frames: usize,
    /// Everything pushed, for sounds shorter than one block.
    total: f64,
    total_frames: usize,
    peak: f32,
}

#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

fn k_weighting(rate: u32) -> [Biquad; 2] {
    let rate = f64::from(rate);
    let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (PI * f0 / rate).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (PI * f0 / rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let high_pass = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    [shelf, high_pass]
}

impl LoudnessMeter {
    pub fn new(rate: u32, channels: usize) -> LoudnessMeter {
        LoudnessMeter {
            channels,
            hop: (rate as usize).div_ceil(10).max(1),
            filters: vec![k_weighting(rate); channels],
            hops: Vec::new(),
            acc: 0.0,
            acc_frames: 0,
            total: 0.0,
            total_frames: 0,
            peak: 0.0,
        }
    }

    /// Adds interleaved frames.
    pub fn push(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.channels) {
            let mut energy = 0.0;
            for (x, filters) in frame.iter().zip(&mut self.filters) {
                self.peak = self.peak.max(x.abs());
                let [shelf, high_pass] = filters;
                let y = high_pass.run(shelf.run(f64::from(*x)));
                energy += y * y;
            }
            self.acc += energy;
            self.total += energy;
            self.acc_frames += 1;
            self.total_frames += 1;
            if self.acc_frames == self.hop {
                self.hops.push(self.acc / self.hop as f64);
                self.acc = 0.0;
                self.acc_frames = 0;
            }
        }
    }

    /// LUFS; −∞ when nothing passes the absolute gate.
    pub fn integrated(&self) -> f32 {
        if self.hops.len() < 4 {
            if self.total_frames == 0 {
                return f32::NEG_INFINITY;
            }
            return lufs(self.total / self.total_frames as f64);
        }
        let blocks: Vec<f64> = self
            .hops
            .windows(4)
            .map(|w| w.iter().sum::<f64>() / 4.0)
            .collect();
        let absolute = energy_of(-70.0);
        let gated = |threshold: f64| {
            let kept: Vec<f64> = blocks.iter().copied().filter(|&z| z > threshold).collect();
            (!kept.is_empty()).then(|| kept.iter().sum::<f64>() / kept.len() as f64)
        };
        let Some(first) = gated(absolute) else {
            return f32::NEG_INFINITY;
        };
        let relative = first * 10f64.powf(-10.0 / 10.0);
        gated(relative.max(absolute)).map_or(f32::NEG_INFINITY, lufs)
    }

    /// Largest sample magnitude, in dBFS.
    pub fn peak_dbfs(&self) -> f32 {
        20.0 * self.peak.log10()
    }
}

fn lufs(energy: f64) -> f32 {
    (-0.691 + 10.0 * energy.log10()) as f32
}

fn energy_of(lufs: f64) -> f64 {
    10f64.powf((lufs + 0.691) / 10.0)
}

/// Integrated loudness of a whole interleaved sound.
pub fn integrated_lufs(samples: &[f32], channels: usize, rate: u32) -> (f32, f32) {
    let mut meter = LoudnessMeter::new(rate, channels);
    meter.push(samples);
    (meter.integrated(), meter.peak_dbfs())
}

/// Level step, in dB, between the `window` frames before the loop end and
/// those after the loop start: what a listener hears jump at every seam.
/// Silence counts as −120 dB.
pub fn seam_step_db(
    samples: &[f32],
    channels: usize,
    start: usize,
    end: usize,
    window: usize,
) -> f32 {
    let window = window.min((end - start) / 2).max(1);
    let mean_square = |from: usize| {
        let s = &samples[from * channels..(from + window) * channels];
        s.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / s.len() as f64
    };
    let db = |ms: f64| 10.0 * ms.max(1e-12).log10();
    (db(mean_square(end - window)) - db(mean_square(start))).abs() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k_weighting_at_48k_matches_bs1770_table() {
        let [shelf, high_pass] = k_weighting(48_000);
        let expect = [1.53512485958697, -2.69169618940638, 1.19839281085285];
        for (b, e) in shelf.b.iter().zip(expect) {
            assert!((b - e).abs() < 1e-6, "{b} {e}");
        }
        assert!((shelf.a[0] - -1.69065929318241).abs() < 1e-6);
        assert!((high_pass.a[1] - 0.99007225036621).abs() < 1e-6);
    }
}
