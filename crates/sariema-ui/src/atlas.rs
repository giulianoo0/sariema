// SPDX-License-Identifier: Apache-2.0

//! The glyph atlas: one channel holding each glyph's signed distance field,
//! so one rasterisation serves every size, outline and soft shadow.
//!
//! The field is computed like Mapbox's TinySDF: a Felzenszwalb–Huttenlocher
//! exact Euclidean distance transform seeded from the antialiased coverage,
//! so edges keep subpixel position. A texel holds `0.5 − d / (2 · spread)`,
//! with `d` the distance to the outline in rasterised pixels, positive
//! outside: the outline is at 0.5.

/// Where a glyph is in the atlas and how it sits on its pen position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    /// Texel rectangle: x, y, width, height.
    pub rect: [u32; 4],
    /// Offset of the rectangle's top-left from the pen position on the
    /// baseline, in rasterised pixels, y down.
    pub bearing: [f32; 2],
}

/// Squared distance standing for "no seed": finite, so the envelope's
/// arithmetic stays finite.
const FAR: f64 = 1e20;

pub struct Atlas {
    pub size: u32,
    pub texels: Vec<u8>,
    spread: f32,
    shelves: Vec<Shelf>,
    /// Texels changed since the renderer last took them.
    pub dirty: bool,
    outer: Vec<f64>,
    inner: Vec<f64>,
    f: Vec<f64>,
    z: Vec<f64>,
    v: Vec<usize>,
}

struct Shelf {
    y: u32,
    height: u32,
    used: u32,
}

impl Atlas {
    pub fn new(size: u32, spread: f32) -> Atlas {
        Atlas {
            size,
            texels: vec![0; size as usize * size as usize],
            spread,
            shelves: Vec::new(),
            dirty: true,
            outer: Vec::new(),
            inner: Vec::new(),
            f: Vec::new(),
            z: Vec::new(),
            v: Vec::new(),
        }
    }

    /// Texels on each side of a glyph's coverage, enough for the field to
    /// fall from the outline to 0.
    fn pad(&self) -> u32 {
        self.spread.ceil() as u32 + 1
    }

    /// Adds a glyph from its coverage (`width × height`, row-major, [0, 1])
    /// whose top-left sits at `origin` from the pen. `None` when the atlas
    /// is full.
    pub fn insert(
        &mut self,
        coverage: &[f32],
        width: u32,
        height: u32,
        origin: [f32; 2],
    ) -> Option<Cell> {
        let pad = self.pad();
        let (w, h) = (width + 2 * pad, height + 2 * pad);
        let (x, y) = self.allocate(w, h)?;
        self.field(coverage, width, height, pad);
        let size = self.size as usize;
        for row in 0..h as usize {
            let dst = (y as usize + row) * size + x as usize;
            let src = row * w as usize;
            for (t, d) in self.texels[dst..dst + w as usize]
                .iter_mut()
                .zip(&self.outer[src..src + w as usize])
            {
                *t = (*d * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        self.dirty = true;
        Some(Cell {
            rect: [x, y, w, h],
            bearing: [origin[0] - pad as f32, origin[1] - pad as f32],
        })
    }

    fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.size {
            return None;
        }
        if let Some(shelf) = self
            .shelves
            .iter_mut()
            .find(|s| h <= s.height && s.used + w <= self.size)
        {
            let x = shelf.used;
            shelf.used += w;
            return Some((x, shelf.y));
        }
        let y = self.shelves.last().map_or(0, |s| s.y + s.height);
        if y + h > self.size {
            return None;
        }
        self.shelves.push(Shelf {
            y,
            height: h,
            used: w,
        });
        Some((0, y))
    }

    /// Leaves the normalised field of the padded glyph in `self.outer`.
    fn field(&mut self, coverage: &[f32], width: u32, height: u32, pad: u32) {
        let (w, h) = ((width + 2 * pad) as usize, (height + 2 * pad) as usize);
        let len = w * h;
        self.outer.clear();
        self.outer.resize(len, FAR);
        self.inner.clear();
        self.inner.resize(len, 0.0);
        for y in 0..height as usize {
            for x in 0..width as usize {
                let a = f64::from(coverage[y * width as usize + x]);
                if a <= 0.0 {
                    continue;
                }
                let j = (y + pad as usize) * w + x + pad as usize;
                if a >= 1.0 {
                    self.outer[j] = 0.0;
                    self.inner[j] = FAR;
                } else {
                    let d = 0.5 - a;
                    self.outer[j] = if d > 0.0 { d * d } else { 0.0 };
                    self.inner[j] = if d < 0.0 { d * d } else { 0.0 };
                }
            }
        }
        let n = w.max(h);
        self.f.resize(n, 0.0);
        self.z.resize(n + 1, 0.0);
        self.v.resize(n, 0);
        edt(&mut self.outer, w, h, &mut self.f, &mut self.z, &mut self.v);
        edt(&mut self.inner, w, h, &mut self.f, &mut self.z, &mut self.v);
        let range = 2.0 * f64::from(self.spread);
        for (o, i) in self.outer.iter_mut().zip(&self.inner) {
            let d = o.sqrt() - i.sqrt();
            *o = 0.5 - d / range;
        }
    }
}

/// Squared Euclidean distance transform of a `w × h` grid in place: columns,
/// then rows.
fn edt(grid: &mut [f64], w: usize, h: usize, f: &mut [f64], z: &mut [f64], v: &mut [usize]) {
    for x in 0..w {
        edt_1d(grid, x, w, h, f, z, v);
    }
    for y in 0..h {
        edt_1d(grid, y * w, 1, w, f, z, v);
    }
}

/// Felzenszwalb & Huttenlocher's lower envelope of parabolas along one line
/// of `len` cells starting at `offset`, `stride` apart.
fn edt_1d(
    grid: &mut [f64],
    offset: usize,
    stride: usize,
    len: usize,
    f: &mut [f64],
    z: &mut [f64],
    v: &mut [usize],
) {
    v[0] = 0;
    z[0] = -FAR;
    z[1] = FAR;
    f[0] = grid[offset];
    let mut k: isize = 0;
    for q in 1..len {
        f[q] = grid[offset + q * stride];
        let mut s;
        loop {
            let r = v[k as usize];
            s = (f[q] - f[r] + (q * q) as f64 - (r * r) as f64) / (2.0 * (q - r) as f64);
            if s <= z[k as usize] {
                k -= 1;
                if k > -1 {
                    continue;
                }
            }
            break;
        }
        k += 1;
        v[k as usize] = q;
        z[k as usize] = s;
        z[k as usize + 1] = FAR;
    }
    let mut k = 0usize;
    for q in 0..len {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let r = v[k];
        let d = q as f64 - r as f64;
        grid[offset + q * stride] = d * d + f[r];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_is_half_on_the_outline_and_falls_away() {
        let mut atlas = Atlas::new(64, 4.0);
        let n = 8u32;
        let coverage = vec![1.0f32; (n * n) as usize];
        let cell = atlas.insert(&coverage, n, n, [0.0, 0.0]).unwrap();
        let pad = atlas.pad();
        assert_eq!(cell.rect, [0, 0, n + 2 * pad, n + 2 * pad]);
        let at = |x: u32, y: u32| f32::from(atlas.texels[(y * 64 + x) as usize]) / 255.0;
        let mid = pad + n / 2;
        assert!(at(mid, mid) > 0.9, "inside");
        // One pixel from the nearest covered one.
        let edge = at(pad - 1, mid);
        assert!((edge - (0.5 - 1.0 / 8.0)).abs() < 0.01, "{edge}");
        assert!(at(0, mid) < at(pad - 1, mid));
    }

    #[test]
    fn full_atlas_refuses() {
        let mut atlas = Atlas::new(32, 2.0);
        let coverage = vec![1.0f32; 100];
        let mut placed = 0;
        while atlas.insert(&coverage, 10, 10, [0.0, 0.0]).is_some() {
            placed += 1;
        }
        assert_eq!(placed, 4, "16 × 16 cells in a 32 × 32 atlas");
    }
}
