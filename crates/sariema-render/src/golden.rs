// SPDX-License-Identifier: Apache-2.0

use image::RgbaImage;

/// Largest channel difference at which two renders still match.
pub const GOLDEN_MAX_DELTA: u8 = 6;
/// Renders also match when at most this fraction of pixels differs by more
/// than [`GOLDEN_MAX_DELTA`].
pub const GOLDEN_MAX_RATIO: f32 = 0.005;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoldenDiff {
    /// Largest difference in any channel of any pixel.
    pub max_channel_delta: u8,
    /// Fraction of pixels with a channel differing by more than [`GOLDEN_MAX_DELTA`].
    pub differing_pixels_ratio: f32,
}

/// `None` if the images differ in size.
pub fn compare_images(a: &RgbaImage, b: &RgbaImage) -> Option<GoldenDiff> {
    if a.dimensions() != b.dimensions() {
        return None;
    }
    let deltas: Vec<u8> = a
        .pixels()
        .zip(b.pixels())
        .map(|(p, q)| (0..4).map(|c| p[c].abs_diff(q[c])).max().unwrap_or(0))
        .collect();
    let differing = deltas.iter().filter(|&&d| d > GOLDEN_MAX_DELTA).count();
    Some(GoldenDiff {
        max_channel_delta: deltas.iter().copied().max().unwrap_or(0),
        differing_pixels_ratio: differing as f32 / deltas.len().max(1) as f32,
    })
}
