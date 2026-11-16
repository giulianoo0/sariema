// SPDX-License-Identifier: Apache-2.0

//! Frames of equal size tiled into one image, left to right, then down.

use image::RgbaImage;

/// The `frames` in rows of `columns`; an empty list gives an empty image.
pub fn tile(frames: &[RgbaImage], columns: u32) -> RgbaImage {
    let Some(first) = frames.first() else {
        return RgbaImage::new(0, 0);
    };
    let (w, h) = first.dimensions();
    let columns = columns.min(frames.len() as u32).max(1);
    let rows = (frames.len() as u32).div_ceil(columns);
    let mut strip = RgbaImage::new(w * columns, h * rows);
    for (i, frame) in (0..).zip(frames) {
        image::imageops::replace(
            &mut strip,
            frame,
            i64::from(i % columns * w),
            i64::from(i / columns * h),
        );
    }
    strip
}

/// `count` times from `start` over `span` seconds, both ends included.
pub fn times(start: f32, span: f32, count: u32) -> Vec<f32> {
    match count {
        0 => Vec::new(),
        1 => vec![start],
        _ => (0..count)
            .map(|i| start + span * i as f32 / (count - 1) as f32)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fill_rows_left_to_right() {
        let frame = |v: u8| RgbaImage::from_pixel(2, 1, image::Rgba([v, 0, 0, 255]));
        let strip = tile(&[frame(1), frame(2), frame(3)], 2);
        assert_eq!(strip.dimensions(), (4, 2));
        assert_eq!(strip.get_pixel(2, 0)[0], 2);
        assert_eq!(strip.get_pixel(0, 1)[0], 3);
        assert_eq!(strip.get_pixel(3, 1)[3], 0);
    }

    #[test]
    fn times_span_both_ends() {
        assert_eq!(times(1.0, 2.0, 3), [1.0, 2.0, 3.0]);
        assert_eq!(times(0.5, 9.0, 1), [0.5]);
    }
}
