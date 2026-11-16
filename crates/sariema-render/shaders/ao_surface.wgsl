// SPDX-License-Identifier: Apache-2.0
// Appended to the shaders that filter the occlusion, whose texels hold the
// fraction of ambient light left and the view depth it was estimated at.

// 1 at equal depths, 0 once they are 2% of `reference` apart: another
// surface. 0 too for a depth of 0, which no surface has.
fn same_surface(depth: f32, reference: f32) -> f32 {
    let tolerance = 0.02 * reference;
    let apart = abs(depth - reference);
    return select(0.0, 1.0 - apart / tolerance, apart < tolerance);
}
