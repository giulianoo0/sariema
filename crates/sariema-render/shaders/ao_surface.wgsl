// SPDX-License-Identifier: Apache-2.0
// Appended to the shaders that filter the occlusion, whose texels hold the
// fraction of ambient light left and the view depth it was estimated at.

// 1 at equal depths, 0 once they are 2% of `reference` apart: another surface.
fn same_surface(depth: f32, reference: f32) -> f32 {
    return saturate(1.0 - abs(depth - reference) / (0.02 * reference));
}
